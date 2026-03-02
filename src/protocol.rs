use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_MESSAGE_SIZE: u32 = 10 * 1024 * 1024; // 10MB

pub async fn read_message<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);

    if len == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero-length message",
        ));
    }
    if len > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len} bytes (max {MAX_MESSAGE_SIZE})"),
        ));
    }

    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

pub async fn write_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> io::Result<()> {
    let len = data.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_roundtrip() {
        let payload = b"{\"method\":\"isRunning\",\"params\":{}}";
        let mut buf = Vec::new();
        write_message(&mut buf, payload).await.unwrap();

        let mut cursor = io::Cursor::new(buf);
        let result = read_message(&mut cursor).await.unwrap();
        assert_eq!(result, payload);
    }

    #[tokio::test]
    async fn test_big_endian_prefix() {
        let payload = b"hello";
        let mut buf = Vec::new();
        write_message(&mut buf, payload).await.unwrap();
        assert_eq!(&buf[..4], &[0, 0, 0, 5]);
    }

    #[tokio::test]
    async fn test_zero_length_rejected() {
        let buf = vec![0u8; 4]; // length = 0
        let mut cursor = io::Cursor::new(buf);
        let result = read_message(&mut cursor).await;
        assert!(result.is_err());
    }
}
