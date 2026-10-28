use super::*;
use serde_json::json;
use std::future::Future;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tokio::io::ReadBuf;

struct ScriptReader {
    chunks: Arc<Mutex<VecDeque<Vec<u8>>>>,
    calls: Arc<AtomicUsize>,
    pending_after: bool,
}
impl ScriptReader {
    fn new(chunks: &[&[u8]], pending_after: bool) -> Self {
        Self {
            chunks: Arc::new(Mutex::new(
                chunks.iter().map(|bytes| bytes.to_vec()).collect(),
            )),
            calls: Arc::new(AtomicUsize::new(0)),
            pending_after,
        }
    }
}
impl AsyncRead for ScriptReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut chunks = self.chunks.lock().unwrap();
        if let Some(chunk) = chunks.front_mut() {
            let count = chunk.len().min(buffer.remaining());
            buffer.put_slice(&chunk[..count]);
            chunk.drain(..count);
            if chunk.is_empty() {
                chunks.pop_front();
            }
            Poll::Ready(Ok(()))
        } else if self.pending_after {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

#[derive(Default)]
struct Written {
    bytes: Vec<u8>,
    flushes: usize,
}
struct CaptureWriter {
    written: Arc<Mutex<Written>>,
    fail_write: bool,
    fail_flush: bool,
}
impl CaptureWriter {
    fn new() -> Self {
        Self {
            written: Arc::default(),
            fail_write: false,
            fail_flush: false,
        }
    }
}
impl AsyncWrite for CaptureWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.fail_write {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let count = bytes.len().min(3);
        self.written
            .lock()
            .unwrap()
            .bytes
            .extend_from_slice(&bytes[..count]);
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.written.lock().unwrap().flushes += 1;
        Poll::Ready(if self.fail_flush {
            Err(std::io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        })
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn fragmented_frames_queue_before_reads_and_clean_eof_is_stable() {
    let reader = ScriptReader::new(
        &[
            b"{\"id\":1",
            b",\"result\":null}\n{\"method\":\"notice\"}\n",
        ],
        false,
    );
    let calls = reader.calls.clone();
    let mut transport = RpcTransport::new(reader, tokio::io::sink());
    assert!(
        matches!(transport.read(None).await.unwrap(), TransportRead::Message(RpcMessage::Response { id, result }) if id == json!(1) && result.is_null())
    );
    assert!(transport.has_queued_messages());
    assert_eq!(transport.pending_bytes(), 0);
    let reads = calls.load(Ordering::SeqCst);
    let count = transport.wire_bytes_read();
    let valid = transport.last_valid_output();
    assert!(
        matches!(transport.read(Some(0)).await.unwrap(), TransportRead::Message(RpcMessage::Notification { method, params }) if method == "notice" && params.is_null())
    );
    assert_eq!(calls.load(Ordering::SeqCst), reads);
    assert_eq!(transport.wire_bytes_read(), count);
    assert_eq!(transport.last_valid_output(), valid);
    assert!(!transport.clean_eof_observed());
    assert!(matches!(
        transport.read(Some(0)).await.unwrap(),
        TransportRead::CleanEof
    ));
    assert!(transport.clean_eof_observed());
    assert!(matches!(
        transport.read(None).await.unwrap(),
        TransportRead::CleanEof
    ));
    assert_eq!(calls.load(Ordering::SeqCst), reads + 1);
}

#[tokio::test]
async fn exact_wire_bound_allows_message_and_zero_bound_distinguishes_eof_from_excess() {
    let frame = b"{\"id\":1,\"result\":null}\n";
    let mut exact = RpcTransport::new(ScriptReader::new(&[frame], false), tokio::io::sink());
    assert!(matches!(
        exact.read(Some(frame.len() as u64)).await.unwrap(),
        TransportRead::Message(_)
    ));
    assert_eq!(exact.wire_bytes_read(), frame.len() as u64);
    assert!(matches!(
        exact.read(Some(0)).await.unwrap(),
        TransportRead::CleanEof
    ));
    for bound in [0, 5] {
        let mut excess =
            RpcTransport::new(ScriptReader::new(&[b"1234567"], false), tokio::io::sink());
        assert_eq!(
            excess.read(Some(bound)).await.err().unwrap(),
            Error::new("bounded_input", "declared tail wire byte bound exceeded")
        );
        assert_eq!(excess.wire_bytes_read(), bound + 1);
        assert_eq!(excess.pending_bytes(), 0);
        assert!(excess.decoder.poisoned);
        assert!(!excess.clean_eof_observed());
        assert_eq!(
            excess.read(None).await.err().unwrap().code,
            "protocol_error"
        );
    }
}

#[tokio::test]
async fn malformed_batch_promotes_nothing_and_unterminated_eof_retains_diagnostic_bytes() {
    let mut malformed = RpcTransport::new(
        ScriptReader::new(&[b"{\"id\":1,\"result\":null}\nnot-json\n"], false),
        tokio::io::sink(),
    );
    let initial = malformed.last_valid_output();
    assert_eq!(
        malformed.read(None).await.err().unwrap().code,
        "protocol_error"
    );
    assert!(!malformed.has_queued_messages());
    assert_eq!(malformed.pending_bytes(), 0);
    assert!(malformed.decoder.poisoned);
    assert_eq!(malformed.last_valid_output(), initial);
    let mut partial =
        RpcTransport::new(ScriptReader::new(&[b"{\"id\":"], false), tokio::io::sink());
    assert_eq!(
        partial.read(None).await.err().unwrap(),
        Error::new(
            "protocol_error",
            "unterminated or invalid app-server frame at exit"
        )
    );
    assert_eq!(partial.pending_bytes(), 6);
    assert_eq!(partial.wire_bytes_read(), 6);
    assert!(partial.decoder.poisoned);
    assert!(!partial.clean_eof_observed());
}

#[tokio::test]
async fn cancelled_partial_read_retains_bytes_and_only_complete_frames_refresh_activity() {
    let reader = ScriptReader::new(&[b"{\"id\":1"], true);
    let chunks = reader.chunks.clone();
    let mut transport = RpcTransport::new(reader, tokio::io::sink());
    let initial = Instant::now() - Duration::from_secs(1);
    transport.last_valid_output = initial;
    let mut pending = Box::pin(transport.read(None));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(pending.as_mut().poll(&mut cx).is_pending());
    drop(pending);
    assert_eq!(transport.pending_bytes(), 7);
    assert_eq!(transport.wire_bytes_read(), 7);
    assert_eq!(transport.last_valid_output(), initial);
    chunks
        .lock()
        .unwrap()
        .push_back(b",\"result\":null}\n".to_vec());
    assert!(matches!(
        transport.read(None).await.unwrap(),
        TransportRead::Message(_)
    ));
    assert_eq!(transport.pending_bytes(), 0);
    assert_eq!(transport.wire_bytes_read(), 23);
    assert!(transport.last_valid_output() > initial);
}

#[tokio::test]
async fn cancelled_outer_read_retains_already_observed_valid_activity() {
    let reader = ScriptReader::new(&[b"{\"id\":1,\"result\":null}\n"], true);
    let mut transport = RpcTransport::new(reader, tokio::io::sink());
    let initial = Instant::now() - Duration::from_secs(1);
    transport.last_valid_output = initial;
    let mut pending = Box::pin(async {
        assert!(matches!(
            transport.read(None).await.unwrap(),
            TransportRead::Message(_)
        ));
        transport.read(None).await
    });
    let mut cx = Context::from_waker(Waker::noop());
    assert!(pending.as_mut().poll(&mut cx).is_pending());
    drop(pending);
    assert!(transport.last_valid_output() > initial);
    assert_eq!(transport.wire_bytes_read(), 23);
    assert!(!transport.has_queued_messages());
}

#[tokio::test]
async fn send_writes_exact_framing_then_flushes_and_refuses_oversize_without_io() {
    let writer = CaptureWriter::new();
    let written = writer.written.clone();
    let mut transport = RpcTransport::new(tokio::io::empty(), writer);
    let initial = transport.last_valid_output();
    transport
        .send(&json!({"id":1,"result":null}))
        .await
        .unwrap();
    assert_eq!(
        written.lock().unwrap().bytes,
        b"{\"id\":1,\"result\":null}\n"
    );
    assert_eq!(written.lock().unwrap().flushes, 1);
    assert_eq!(transport.last_valid_output(), initial);
    let before = written.lock().unwrap().bytes.clone();
    assert_eq!(
        transport
            .send(&json!("x".repeat(MAX_FRAME_BYTES)))
            .await
            .unwrap_err(),
        Error::new("protocol_error", "outgoing frame exceeds 10 MiB")
    );
    assert_eq!(written.lock().unwrap().bytes, before);
    assert_eq!(written.lock().unwrap().flushes, 1);
}

#[tokio::test]
async fn write_and_flush_errors_remain_distinct_without_resends() {
    for fail_write in [true, false] {
        let mut writer = CaptureWriter::new();
        writer.fail_write = fail_write;
        writer.fail_flush = !fail_write;
        let written = writer.written.clone();
        let mut transport = RpcTransport::new(tokio::io::empty(), writer);
        let error = transport.send(&json!(null)).await.unwrap_err();
        let context = if fail_write {
            "write app-server protocol"
        } else {
            "flush app-server protocol"
        };
        assert_eq!(
            error,
            Error::io(context, std::io::ErrorKind::BrokenPipe.into())
        );
        let written = written.lock().unwrap();
        assert_eq!(written.flushes, usize::from(!fail_write));
        assert_eq!(
            written.bytes,
            if fail_write {
                b"".as_slice()
            } else {
                b"null\n".as_slice()
            }
        );
    }
}
