//! Reading tool output with fixed memory limits. yt-dlp's output is untrusted in size, so
//! these readers keep memory bounded while still draining the pipe (a child blocked on a
//! full pipe would never exit).

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

/// Largest metadata document accepted from a lookup.
pub const MAX_METADATA: usize = 32 * 1024 * 1024;
/// Error output kept for messages: the last part, where the final error is.
pub const DIAGNOSTIC_TAIL: usize = 256 * 1024;
/// Longest structured line (a progress or marker line) accepted.
pub const MAX_LINE: usize = 256 * 1024;

/// Reads everything up to `limit` bytes. `Err(read)` means the stream went past the
/// limit: reading stopped there, and the caller should end the process.
pub async fn read_capped<R: AsyncRead + Unpin>(mut stream: R, limit: usize) -> Result<Vec<u8>, usize> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return Ok(out),
            Ok(n) => {
                if out.len() + n > limit {
                    return Err(out.len() + n);
                }
                out.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

/// Reads the whole stream, keeping only its last `limit` bytes.
pub async fn read_tail<R: AsyncRead + Unpin>(mut stream: R, limit: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    while let Ok(n) = stream.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
        // Trimmed in batches, so a long stream is not shifted on every read.
        if out.len() > limit * 2 {
            out.drain(..out.len() - limit);
        }
    }
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

/// One line into `buf` (without its newline), at most `limit` bytes of it. The rest of an
/// overlong line is read and discarded, so the pipe keeps draining. Returns `None` at the
/// end of the stream, and `Some(true)` when the line was cut.
pub async fn read_line_capped<R: AsyncBufRead + Unpin>(reader: &mut R, buf: &mut Vec<u8>, limit: usize) -> Option<bool> {
    buf.clear();
    let mut cut = false;
    let mut any = false;
    loop {
        let available = match reader.fill_buf().await {
            Ok(bytes) => bytes,
            Err(_) => return any.then_some(cut),
        };
        if available.is_empty() {
            return any.then_some(cut);
        }
        any = true;
        let (piece, used, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (&available[..i], i + 1, true),
            None => (available, available.len(), false),
        };
        let room = limit.saturating_sub(buf.len());
        if piece.len() > room {
            cut = true;
        }
        buf.extend_from_slice(&piece[..piece.len().min(room)]);
        reader.consume(used);
        if done {
            return Some(cut);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn just_below_and_just_above_the_cap() {
        rt().block_on(async {
            let data = vec![b'x'; 1000];
            assert_eq!(read_capped(&data[..], 1000).await, Ok(data.clone()));
            assert!(read_capped(&data[..], 999).await.is_err());
        });
    }

    #[test]
    fn the_tail_keeps_a_late_error_after_noise() {
        rt().block_on(async {
            let mut text = "noise line\n".repeat(100_000).into_bytes();
            text.extend_from_slice(b"ERROR: the real reason\n");
            let tail = read_tail(&text[..], 4096).await;
            assert_eq!(tail.len(), 4096);
            assert!(String::from_utf8_lossy(&tail).ends_with("ERROR: the real reason\n"));
            assert_eq!(read_tail(&b"short"[..], 4096).await, b"short");
        });
    }

    #[test]
    fn an_endless_line_is_cut_and_the_next_line_still_read() {
        rt().block_on(async {
            let (mut writer, reader) = tokio::io::duplex(1024);
            let producer = tokio::spawn(async move {
                // 3 MiB without a newline, then two normal lines.
                for _ in 0..(3 * 1024) {
                    writer.write_all(&[b'a'; 1024]).await.unwrap();
                }
                writer.write_all(b"\nCLAWS|after\nlast").await.unwrap();
            });
            let mut reader = BufReader::new(reader);
            let mut buf = Vec::new();
            assert_eq!(read_line_capped(&mut reader, &mut buf, 1000).await, Some(true));
            assert_eq!(buf.len(), 1000);
            assert_eq!(read_line_capped(&mut reader, &mut buf, 1000).await, Some(false));
            assert_eq!(buf, b"CLAWS|after");
            assert_eq!(read_line_capped(&mut reader, &mut buf, 1000).await, Some(false));
            assert_eq!(buf, b"last");
            assert_eq!(read_line_capped(&mut reader, &mut buf, 1000).await, None);
            producer.await.unwrap();
        });
    }

    #[test]
    fn an_oversized_answer_stops_reading_and_the_process_can_be_ended() {
        rt().block_on(async {
            // A child writing 64 MiB, far past a 1 MiB cap: it blocks once nobody reads.
            let Ok(mut child) = tokio::process::Command::new("python")
                .args(["-c", "import sys; sys.stdout.buffer.write(b'x' * (64 << 20))"])
                .stdout(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            else {
                eprintln!("skipped: needs Python");
                return;
            };
            let started = std::time::Instant::now();
            let read = read_capped(child.stdout.take().unwrap(), 1024 * 1024).await;
            assert!(read.is_err(), "the cap must trip");
            if let Some(pid) = child.id() {
                crate::downloader::kill_tree(pid, true);
            }
            // Generous bounds: on a loaded CI runner, starting Python and taskkill can each
            // take seconds. A reader that ignored the cap or a child that survived would hang.
            let limit = std::time::Duration::from_secs(60);
            let _ = tokio::time::timeout(limit, child.wait()).await.expect("the process must end");
            assert!(started.elapsed() < limit, "took {:?}", started.elapsed());
        });
    }

    #[test]
    fn crlf_and_utf8_lines_come_through_whole() {
        rt().block_on(async {
            let mut reader = BufReader::new(&b"CLAWSDONE|C:\\V\xc3\xaddeos\\a.mp4\r\n"[..]);
            let mut buf = Vec::new();
            assert_eq!(read_line_capped(&mut reader, &mut buf, MAX_LINE).await, Some(false));
            assert_eq!(String::from_utf8_lossy(&buf).trim_end(), "CLAWSDONE|C:\\Vídeos\\a.mp4");
        });
    }
}
