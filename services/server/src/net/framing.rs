//! Length-prefixed frames on a QUIC stream: a 4-byte little-endian length,
//! then that many bytes of one encoded protocol message.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest accepted frame. Protocol messages are small (a path update of a
/// long ride is a few kB); anything near this is a broken or hostile peer.
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds the limit of {MAX_FRAME}")]
    TooLarge(usize),
    #[error("stream ended inside a frame")]
    Truncated,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The frame header for a payload.
pub fn header(len: usize) -> Result<[u8; 4], FrameError> {
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    Ok((len as u32).to_le_bytes())
}

/// One whole frame: header and payload.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(&header(payload.len())?);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Reads the next frame. `Ok(None)`: the stream ended cleanly between frames.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    let mut head = [0u8; 4];
    // The first byte tells a clean end (nothing) from a cut-off header.
    if r.read(&mut head[..1]).await? == 0 {
        return Ok(None);
    }
    read_exact(r, &mut head[1..]).await?;
    let len = u32::from_le_bytes(head) as usize;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    read_exact(r, &mut buf).await?;
    Ok(Some(buf))
}

async fn read_exact<R: AsyncRead + Unpin>(r: &mut R, buf: &mut [u8]) -> Result<(), FrameError> {
    match r.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(FrameError::Truncated),
        Err(e) => Err(e.into()),
    }
}

/// Writes one frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    payload: &[u8],
) -> Result<(), FrameError> {
    w.write_all(&encode_frame(payload)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn frames_round_trip_back_to_back() {
        let mut bytes = Vec::new();
        for p in [&b"hello"[..], b"", &[7u8; 3000]] {
            bytes.extend(encode_frame(p).unwrap());
        }
        assert_eq!(&bytes[..4], &5u32.to_le_bytes());
        block_on(async {
            let mut r = &bytes[..];
            assert_eq!(read_frame(&mut r).await.unwrap().unwrap(), b"hello");
            assert_eq!(read_frame(&mut r).await.unwrap().unwrap(), b"");
            assert_eq!(read_frame(&mut r).await.unwrap().unwrap(), vec![7u8; 3000]);
            assert!(read_frame(&mut r).await.unwrap().is_none());
        });
    }

    #[test]
    fn frames_survive_arbitrary_chunking() {
        block_on(async {
            let (mut a, mut b) = tokio::io::duplex(7);
            let writer = tokio::spawn(async move {
                for i in 0..50u8 {
                    write_frame(&mut a, &vec![i; usize::from(i) * 13])
                        .await
                        .unwrap();
                }
            });
            for i in 0..50u8 {
                let f = read_frame(&mut b).await.unwrap().unwrap();
                assert_eq!(f, vec![i; usize::from(i) * 13]);
            }
            writer.await.unwrap();
            assert!(read_frame(&mut b).await.unwrap().is_none());
        });
    }

    #[test]
    fn oversized_and_truncated_frames_are_errors() {
        assert!(matches!(
            encode_frame(&vec![0; MAX_FRAME + 1]),
            Err(FrameError::TooLarge(_))
        ));
        block_on(async {
            let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
            assert!(matches!(
                read_frame(&mut &huge[..]).await,
                Err(FrameError::TooLarge(_))
            ));
            // Header cut off.
            assert!(matches!(
                read_frame(&mut &[5u8, 0][..]).await,
                Err(FrameError::Truncated)
            ));
            // Payload cut off.
            let mut short = encode_frame(b"hello").unwrap();
            short.truncate(7);
            assert!(matches!(
                read_frame(&mut &short[..]).await,
                Err(FrameError::Truncated)
            ));
        });
    }
}
