//! Literal fixtures for the generic wire format shipped in 0.13.1.
#![allow(clippy::unwrap_used)]
use serde::Serialize;

fn encoded<T: Serialize>(value: T) -> Vec<u8> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
}

#[tokio::test]
async fn golden_bodies_and_frames() {
    #[derive(Serialize)]
    struct Body {
        id: u32,
        name: &'static str,
    }
    let fixtures = [
        (Vec::new(), vec![0, 0, 0, 0]),
        (b"raw".to_vec(), vec![0, 0, 0, 3, b'r', b'a', b'w']),
        (encoded(1000u32), vec![0, 0, 0, 3, 251, 232, 3]),
        (
            encoded(Body { id: 99, name: "ab" }),
            vec![0, 0, 0, 4, 99, 2, b'a', b'b'],
        ),
        (
            encoded(Ok::<_, &str>("hi")),
            vec![0, 0, 0, 4, 0, 2, b'h', b'i'],
        ),
        (
            encoded(Err::<(), _>("fail")),
            vec![0, 0, 0, 6, 1, 4, b'f', b'a', b'i', b'l'],
        ),
    ];
    for (payload, expected) in fixtures {
        let mut wire = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        wire.extend(payload);
        assert_eq!(wire, expected);
        let mut writer =
            oinq::frame::FramedWriter::new(Vec::new(), oinq::frame::FrameLimit::new(64).unwrap());
        writer.start_raw(&wire[4..]).unwrap();
        writer.flush().await.unwrap();
        assert_eq!(writer.into_inner(), expected);
    }
    let mut request = 0x1234_5678u32.to_le_bytes().to_vec();
    request.extend(encoded(1000u32));
    assert_eq!(request, [0x78, 0x56, 0x34, 0x12, 251, 232, 3]);
    assert_eq!(
        oinq::request::parse_args::<u32>(&request[4..]).unwrap(),
        1000
    );
}
