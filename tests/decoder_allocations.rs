//! Allocation-sensitive coverage of the owned serde decoding paths.
#![allow(clippy::unwrap_used)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    fmt,
};

use oinq::frame::{Frame, FrameError, FrameLimit, FramedReader};
use serde::{Deserialize, Deserializer, de::Visitor};

struct TrackingAllocator;

#[derive(Clone, Copy)]
struct Allocations {
    sizes: [usize; 128],
    count: usize,
}

thread_local! {
    // Track only this test thread and only while decoding, excluding setup and
    // the test harness. A fixed array keeps the tracker allocation-free.
    static ALLOCATIONS: Cell<Option<Allocations>> = const { Cell::new(None) };
}

fn record(size: usize) {
    let _ = ALLOCATIONS.try_with(|slot| {
        if let Some(mut allocations) = slot.get() {
            if allocations.count < allocations.sizes.len() {
                allocations.sizes[allocations.count] = size;
            }
            allocations.count += 1;
            slot.set(Some(allocations));
        }
    });
}

// SAFETY: All operations delegate to System with the original arguments.
// Tracking uses only thread-local, allocation-free state.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn tracked<T>(decode: impl FnOnce() -> T) -> (T, Vec<usize>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.with(|slot| slot.set(None));
        }
    }

    ALLOCATIONS.with(|slot| {
        assert!(slot.get().is_none());
        slot.set(Some(Allocations {
            sizes: [0; 128],
            count: 0,
        }));
    });
    let reset = Reset;
    let result = decode();
    let allocations = ALLOCATIONS.with(|slot| slot.replace(None).unwrap());
    drop(reset);
    assert!(
        allocations.count <= allocations.sizes.len(),
        "tracker overflow"
    );
    (result, allocations.sizes[..allocations.count].to_vec())
}

fn frame(payload: &[u8], limit: usize) -> Frame {
    let mut wire = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    wire.extend_from_slice(payload);
    futures::executor::block_on(
        FramedReader::new(wire.as_slice(), FrameLimit::new(limit).unwrap()).read(),
    )
    .unwrap()
    .unwrap()
}

// These visitors require owned callbacks, as permitted by serde. In particular,
// Vec<u8>'s ordinary sequence path does not exercise deserialize_byte_buf.
#[derive(Debug, PartialEq)]
struct OwnedBytes(Vec<u8>);

impl<'de> Deserialize<'de> for OwnedBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl Visitor<'_> for BytesVisitor {
            type Value = OwnedBytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an owned byte buffer")
            }
            fn visit_byte_buf<E: serde::de::Error>(self, bytes: Vec<u8>) -> Result<Self::Value, E> {
                Ok(OwnedBytes(bytes))
            }
        }
        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

#[derive(Debug, PartialEq)]
struct OwnedString(String);

impl<'de> Deserialize<'de> for OwnedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;
        // Deliberately require the owned callback to verify it is preserved.
        #[allow(clippy::serde_api_misuse)]
        impl Visitor<'_> for StringVisitor {
            type Value = OwnedString;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an owned string")
            }
            fn visit_string<E: serde::de::Error>(self, string: String) -> Result<Self::Value, E> {
                Ok(OwnedString(string))
            }
        }
        deserializer.deserialize_string(StringVisitor)
    }
}

fn rejects_without_declared_allocation<T: for<'de> Deserialize<'de>>(payload: &[u8]) {
    let frame = frame(payload, 64);
    for strict in [false, true] {
        let (result, sizes) = tracked(|| {
            if strict {
                frame.decode_strict::<T>()
            } else {
                frame.decode::<T>()
            }
        });
        assert!(matches!(result, Err(FrameError::Decode(_))));
        assert!(
            sizes.iter().all(|&size| size < 1024),
            "allocations: {sizes:?}"
        );
    }
}

#[test]
fn oversized_owned_lengths_do_not_allocate_before_truncation() {
    // Standard bincode varint for 1 MiB, with no actual string/byte data.
    let declared = [252, 0, 0, 16, 0];
    rejects_without_declared_allocation::<String>(&declared);
    rejects_without_declared_allocation::<OwnedString>(&declared);
    rejects_without_declared_allocation::<OwnedBytes>(&declared);

    let nested = [1, 252, 0, 0, 16, 0];
    rejects_without_declared_allocation::<Vec<String>>(&nested);
    rejects_without_declared_allocation::<Vec<OwnedBytes>>(&nested);
    rejects_without_declared_allocation::<Option<OwnedString>>(&nested);
    rejects_without_declared_allocation::<Option<OwnedBytes>>(&nested);
}

fn rejects_before_consuming_remaining_budget<T: for<'de> Deserialize<'de>>() {
    // A four-byte string followed by a vector containing one owned value.
    // Account for the preceding string and the vector element's storage.
    let length = 64 - 4 - std::mem::size_of::<T>() + 1;
    let mut payload = vec![
        4,
        b'a',
        b'b',
        b'c',
        b'd',
        1,
        u8::try_from(length).expect("should not exceed 64"),
    ];
    payload.extend(std::iter::repeat_n(b'x', length));
    let over_budget = frame(&payload, 64);
    for strict in [false, true] {
        let (result, sizes) = tracked(|| {
            if strict {
                over_budget.decode_strict::<(String, Vec<T>)>()
            } else {
                over_budget.decode::<(String, Vec<T>)>()
            }
        });
        assert!(matches!(result, Err(FrameError::Decode(_))));
        assert!(sizes.contains(&4), "first string should be decoded");
        assert!(
            !sizes.contains(&length),
            "over-budget allocation: {sizes:?}"
        );
    }

    // One fewer byte exactly exhausts the shared budget and must still work.
    payload[6] -= 1;
    payload.pop();
    let exact = frame(&payload, 64);
    let (result, sizes) = tracked(|| exact.decode_strict::<(String, Vec<T>)>());
    assert!(result.is_ok());
    assert!(sizes.contains(&(length - 1)));
}

#[test]
fn nested_owned_values_check_remaining_budget_before_allocating() {
    rejects_before_consuming_remaining_budget::<String>();
    rejects_before_consuming_remaining_budget::<OwnedString>();
    rejects_before_consuming_remaining_budget::<OwnedBytes>();
}

#[test]
fn owned_callbacks_and_borrowed_decoding_still_work() {
    for length in [0u8, 63] {
        let mut payload = vec![length];
        payload.extend(std::iter::repeat_n(b'x', usize::from(length)));
        let frame = frame(&payload, 64);
        let expected = "x".repeat(usize::from(length));
        assert_eq!(frame.decode_strict::<String>().unwrap(), expected);
        assert_eq!(frame.decode_strict::<OwnedString>().unwrap().0, expected);
        assert_eq!(
            frame.decode_strict::<OwnedBytes>().unwrap().0,
            expected.as_bytes()
        );
        assert_eq!(frame.decode_strict::<&str>().unwrap(), expected);
        assert_eq!(frame.decode_strict::<&[u8]>().unwrap(), expected.as_bytes());
    }
}
