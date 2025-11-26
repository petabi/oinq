//! Helper functions for request handlers.

use std::io;

use quinn::SendStream;
use serde::{Deserialize, Serialize};

use crate::{frame, message};

/// Parses the arguments of a request.
///
/// # Errors
///
/// Returns an error if the arguments could not be deserialized.
pub fn parse_args<'de, T: Deserialize<'de>>(args: &'de [u8]) -> io::Result<T> {
    bincode::serde::borrow_decode_from_slice(args, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        .map(|(value, _)| value)
}

/// Sends a response to a request.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send_response<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> io::Result<()> {
    if let Err(e) = frame::send(send, buf, body).await {
        if e.kind() == io::ErrorKind::InvalidData {
            message::send_err(send, buf, e).await
        } else {
            Err(e)
        }
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::parse_args;

    #[test]
    fn parse_empty_string() {
        let data: &[u8] = &[0x00];
        let result: &str = parse_args(data).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn parse_short_string() {
        // "hello": length 5 + UTF-8 bytes
        let data: &[u8] = &[0x05, b'h', b'e', b'l', b'l', b'o'];
        let result: &str = parse_args(data).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn parse_owned_string() {
        let data: &[u8] = &[0x05, b'w', b'o', b'r', b'l', b'd'];
        let result: String = parse_args(data).unwrap();
        assert_eq!(result, "world");
    }

    #[test]
    fn parse_unicode_string() {
        // "日本語" (9 UTF-8 bytes)
        let data: &[u8] = &[0x09, 0xE6, 0x97, 0xA5, 0xE6, 0x9C, 0xAC, 0xE8, 0xAA, 0x9E];
        let result: &str = parse_args(data).unwrap();
        assert_eq!(result, "日本語");
    }

    #[test]
    fn parse_long_string() {
        // 300-byte string: 0xFB prefix + [0x2C, 0x01] (300 in LE)
        let mut data = vec![0xFB, 0x2C, 0x01];
        data.extend(std::iter::repeat_n(b'x', 300));
        let result: String = parse_args(&data).unwrap();
        assert_eq!(result, "x".repeat(300));
    }

    #[test]
    fn parse_unit() {
        let data: &[u8] = &[];
        let result: () = parse_args(data).unwrap();
        assert_eq!(result, ());
    }

    #[test]
    fn parse_u8() {
        let data: &[u8] = &[0x2A];
        let result: u8 = parse_args(data).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn parse_u16_small() {
        let data: &[u8] = &[0x64]; // 100
        let result: u16 = parse_args(data).unwrap();
        assert_eq!(result, 100);
    }

    #[test]
    fn parse_u16() {
        // 1000: 0xFB prefix + [0xE8, 0x03]
        let data: &[u8] = &[0xFB, 0xE8, 0x03];
        let result: u16 = parse_args(data).unwrap();
        assert_eq!(result, 1000);
    }

    #[test]
    fn parse_u32_small() {
        let data: &[u8] = &[0x7B]; // 123
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 123);
    }

    #[test]
    fn parse_u32() {
        // 0x1234: 0xFB prefix + [0x34, 0x12]
        let data: &[u8] = &[0xFB, 0x34, 0x12];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 0x1234);
    }

    #[test]
    fn parse_u32_large() {
        // 0xDEADBEEF: 0xFC prefix + 4 bytes LE
        let data: &[u8] = &[0xFC, 0xEF, 0xBE, 0xAD, 0xDE];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 0xDEAD_BEEF);
    }

    #[test]
    fn parse_u64() {
        // 0x123456789ABCDEF0: 0xFD prefix + 8 bytes LE
        let data: &[u8] = &[0xFD, 0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12];
        let result: u64 = parse_args(data).unwrap();
        assert_eq!(result, 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn parse_i32_positive() {
        // zigzag: 42 * 2 = 84
        let data: &[u8] = &[0x54];
        let result: i32 = parse_args(data).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn parse_i32_negative() {
        // zigzag: |-1| * 2 - 1 = 1
        let data: &[u8] = &[0x01];
        let result: i32 = parse_args(data).unwrap();
        assert_eq!(result, -1);
    }

    #[test]
    fn parse_i64_negative() {
        // zigzag: |-100| * 2 - 1 = 199
        let data: &[u8] = &[0xC7];
        let result: i64 = parse_args(data).unwrap();
        assert_eq!(result, -100);
    }

    #[test]
    fn parse_bool() {
        assert!(parse_args::<bool>(&[0x01]).unwrap());
        assert!(!parse_args::<bool>(&[0x00]).unwrap());
    }

    #[test]
    fn parse_u32_boundary_250() {
        let data: &[u8] = &[0xFA];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 250);
    }

    #[test]
    fn parse_u32_boundary_251() {
        // 251 needs 0xFB prefix
        let data: &[u8] = &[0xFB, 0xFB, 0x00];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 251);
    }

    #[test]
    fn parse_u32_boundary_65535() {
        let data: &[u8] = &[0xFB, 0xFF, 0xFF];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 65535);
    }

    #[test]
    fn parse_u32_boundary_65536() {
        // Requires 0xFC prefix (4 bytes)
        let data: &[u8] = &[0xFC, 0x00, 0x00, 0x01, 0x00];
        let result: u32 = parse_args(data).unwrap();
        assert_eq!(result, 65536);
    }

    #[test]
    fn parse_result_ok_unit() {
        let data: &[u8] = &[0x00];
        let result: Result<(), &str> = parse_args(data).unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn parse_result_ok_string() {
        let data: &[u8] = &[0x00, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let result: Result<&str, &str> = parse_args(data).unwrap();
        assert_eq!(result, Ok("hello"));
    }

    #[test]
    fn parse_result_err_string() {
        let data: &[u8] = &[0x01, 0x05, b'e', b'r', b'r', b'o', b'r'];
        let result: Result<(), &str> = parse_args(data).unwrap();
        assert_eq!(result, Err("error"));
    }

    #[test]
    fn parse_result_owned_strings() {
        let ok_data: &[u8] = &[0x00, 0x04, b't', b'e', b's', b't'];
        let result: Result<String, String> = parse_args(ok_data).unwrap();
        assert_eq!(result, Ok(String::from("test")));

        let err_data: &[u8] = &[0x01, 0x04, b'f', b'a', b'i', b'l'];
        let result: Result<(), String> = parse_args(err_data).unwrap();
        assert_eq!(result, Err(String::from("fail")));
    }

    #[test]
    fn parse_option_none() {
        let data: &[u8] = &[0x00];
        let result: Option<&str> = parse_args(data).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn parse_option_some_string() {
        let data: &[u8] = &[0x01, 0x05, b'v', b'a', b'l', b'u', b'e'];
        let result: Option<&str> = parse_args(data).unwrap();
        assert_eq!(result, Some("value"));
    }

    #[test]
    fn parse_option_some_u32() {
        // Some(1000): tag 1 + varint 1000
        let data: &[u8] = &[0x01, 0xFB, 0xE8, 0x03];
        let result: Option<u32> = parse_args(data).unwrap();
        assert_eq!(result, Some(1000));
    }

    #[test]
    fn parse_empty_vec() {
        let data: &[u8] = &[0x00];
        let result: Vec<u8> = parse_args(data).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_vec_u8() {
        let data: &[u8] = &[0x03, 0x01, 0x02, 0x03];
        let result: Vec<u8> = parse_args(data).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn parse_vec_u32() {
        // [100, 200, 300]: length 3, then varint elements
        let data: &[u8] = &[0x03, 0x64, 0xC8, 0xFB, 0x2C, 0x01];
        let result: Vec<u32> = parse_args(data).unwrap();
        assert_eq!(result, vec![100, 200, 300]);
    }

    #[test]
    fn parse_vec_string() {
        let data: &[u8] = &[0x02, 0x01, b'a', 0x02, b'b', b'b'];
        let result: Vec<String> = parse_args(data).unwrap();
        assert_eq!(result, vec!["a", "bb"]);
    }

    #[test]
    fn parse_tuple_2() {
        let data: &[u8] = &[0x2A, 0x02, b'h', b'i'];
        let result: (u32, &str) = parse_args(data).unwrap();
        assert_eq!(result, (42, "hi"));
    }

    #[test]
    fn parse_tuple_3() {
        let data: &[u8] = &[0x01, 0xFF, 0x01, b'x'];
        let result: (bool, u8, &str) = parse_args(data).unwrap();
        assert_eq!(result, (true, 255, "x"));
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct SimpleStruct {
        id: u32,
        name: String,
    }

    #[test]
    fn parse_simple_struct() {
        // id = 123, name = "test"
        let data: &[u8] = &[0x7B, 0x04, b't', b'e', b's', b't'];
        let result: SimpleStruct = parse_args(data).unwrap();
        assert_eq!(
            result,
            SimpleStruct {
                id: 123,
                name: String::from("test")
            }
        );
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct NestedStruct {
        inner: SimpleStruct,
        flag: bool,
    }

    #[test]
    fn parse_nested_struct() {
        let data: &[u8] = &[0x01, 0x01, b'a', 0x01];
        let result: NestedStruct = parse_args(data).unwrap();
        assert_eq!(
            result,
            NestedStruct {
                inner: SimpleStruct {
                    id: 1,
                    name: String::from("a")
                },
                flag: true
            }
        );
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct StructWithOption {
        value: Option<u32>,
        label: String,
    }

    #[test]
    fn parse_struct_with_option() {
        // Some(42)
        let some_data: &[u8] = &[0x01, 0x2A, 0x02, b'o', b'k'];
        let result: StructWithOption = parse_args(some_data).unwrap();
        assert_eq!(
            result,
            StructWithOption {
                value: Some(42),
                label: String::from("ok")
            }
        );

        // None
        let none_data: &[u8] = &[0x00, 0x02, b'n', b'o'];
        let result: StructWithOption = parse_args(none_data).unwrap();
        assert_eq!(
            result,
            StructWithOption {
                value: None,
                label: String::from("no")
            }
        );
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum SimpleEnum {
        A,
        B,
        C,
    }

    #[test]
    fn parse_enum_variants() {
        assert_eq!(parse_args::<SimpleEnum>(&[0x00]).unwrap(), SimpleEnum::A);
        assert_eq!(parse_args::<SimpleEnum>(&[0x01]).unwrap(), SimpleEnum::B);
        assert_eq!(parse_args::<SimpleEnum>(&[0x02]).unwrap(), SimpleEnum::C);
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum DataEnum {
        Empty,
        WithU32(u32),
        WithString(String),
        WithBoth { id: u32, name: String },
    }

    #[test]
    fn parse_data_enum() {
        // Empty
        assert_eq!(parse_args::<DataEnum>(&[0x00]).unwrap(), DataEnum::Empty);

        // WithU32(500): variant 1 + 0xFB prefix + [0xF4, 0x01]
        let data: &[u8] = &[0x01, 0xFB, 0xF4, 0x01];
        assert_eq!(
            parse_args::<DataEnum>(data).unwrap(),
            DataEnum::WithU32(500)
        );

        // WithString("msg")
        let data: &[u8] = &[0x02, 0x03, b'm', b's', b'g'];
        assert_eq!(
            parse_args::<DataEnum>(data).unwrap(),
            DataEnum::WithString(String::from("msg"))
        );

        // WithBoth { id: 99, name: "nm" }
        let data: &[u8] = &[0x03, 0x63, 0x02, b'n', b'm'];
        assert_eq!(
            parse_args::<DataEnum>(data).unwrap(),
            DataEnum::WithBoth {
                id: 99,
                name: String::from("nm")
            }
        );
    }
}
