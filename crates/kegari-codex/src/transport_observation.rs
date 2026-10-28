//! Declared-evaluation diagnostics on the original transport. No tool dispatch,
//! pipe writes, native exit proof, permission renewal or complete-channel claim.
use super::*;
use crate::TransportRead;

/// A diagnostic created only by reading this actual client's transport. Its
/// serialized projection is ordinary report data, never a native capability.
pub struct ProtocolTransportEnd {
    diagnostic: ProtocolTransportEndDiagnostic,
}
impl ProtocolTransportEnd {
    pub(super) fn diagnostic(&self) -> &ProtocolTransportEndDiagnostic {
        &self.diagnostic
    }
}
pub(super) struct ObservationOnlyDeny<'a> {
    observer: &'a mut dyn ToolBroker,
}
impl<'a> ObservationOnlyDeny<'a> {
    pub(super) fn new(observer: &'a mut dyn ToolBroker) -> Self {
        Self { observer }
    }
}
impl ToolBroker for ObservationOnlyDeny<'_> {
    fn observe_protocol(&mut self, event: ProtocolObservation<'_>) {
        self.observer.observe_protocol(event);
    }
    fn call<'a>(
        &'a mut self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        // The old bootstrap/interrupt broker denied every call. Never forward
        // this method or session checkpoints to the worker's authority path.
        Box::pin(async move { DenyTools.call(call).await })
    }
}
const MAX_TAIL_FRAMES: u64 = 4096;
const MAX_TAIL_BYTES: u64 = 8 * 1024 * 1024;
fn incomplete(frames: u64, bytes: u64, wire_bytes: u64, error: Error) -> ProtocolTransportEnd {
    ProtocolTransportEnd {
        diagnostic: ProtocolTransportEndDiagnostic::Incomplete {
            frames,
            decoded_bytes: bytes,
            wire_bytes,
            error,
        },
    }
}
fn frame_bytes(message: &RpcMessage) -> Result<u64> {
    // Encoded-frame size, not a claim about original wire whitespace. This also
    // bounds already-decoded queued frames, which precede subsequent reads.
    let value = match message {
        RpcMessage::Response { id, result } => json!({"id":id,"result":result}),
        RpcMessage::Failure { id, error } => json!({"id":id,"error":error}),
        RpcMessage::Notification { method, params } => json!({"method":method,"params":params}),
        RpcMessage::Request { id, method, params } => {
            json!({"id":id,"method":method,"params":params})
        }
    };
    serde_json::to_vec(&value)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| Error::new("protocol_observation", "cannot measure tail frame"))
}
impl RpcClient {
    pub async fn observe_declared_tail(
        &mut self,
        state: &mut SessionState,
        observer: &mut dyn ToolBroker,
        deadline: Instant,
    ) -> ProtocolTransportEnd {
        if self.tail_observation_started {
            return incomplete(
                0,
                0,
                0,
                Error::new(
                    "ownership_conflict",
                    "declared transport tail observation is single-use",
                ),
            );
        }
        self.tail_observation_started = true;
        let mut frames = 0;
        let mut bytes = 0_u64;
        // Residual bytes were really read by this client before the tail began.
        // Include them in the wire budget; queued decoded frames are bounded
        // separately because their original whitespace is no longer retained.
        let residual = self.transport.pending_bytes() as u64;
        let before_wire = self.transport.wire_bytes_read().saturating_sub(residual);
        if residual > MAX_TAIL_BYTES {
            observer.observe_protocol(ProtocolObservation::Issue {
                state: Some(state),
                issue: ProtocolCaptureIssue::CaptureBound,
            });
            return incomplete(
                0,
                0,
                residual,
                Error::new("bounded_input", "buffered tail wire byte bound exceeded"),
            );
        }
        loop {
            if Instant::now() >= deadline {
                return incomplete(
                    frames,
                    bytes,
                    self.transport.wire_bytes_read() - before_wire,
                    Error::new("timeout", "original protocol collection deadline expired"),
                );
            }
            let incoming = match timeout_at(
                deadline,
                self.read_transport_bounded(Some(
                    MAX_TAIL_BYTES.saturating_sub(self.transport.wire_bytes_read() - before_wire),
                )),
            )
            .await
            {
                Err(_) => {
                    return incomplete(
                        frames,
                        bytes,
                        self.transport.wire_bytes_read() - before_wire,
                        Error::new("timeout", "original protocol collection deadline expired"),
                    );
                }
                Ok(Err(error)) => {
                    if error.code == "bounded_input" {
                        observer.observe_protocol(ProtocolObservation::Issue {
                            state: Some(state),
                            issue: ProtocolCaptureIssue::CaptureBound,
                        });
                    }
                    return incomplete(
                        frames,
                        bytes,
                        self.transport.wire_bytes_read() - before_wire,
                        error,
                    );
                }
                Ok(Ok(incoming)) => incoming,
            };
            let message = match incoming {
                TransportRead::CleanEof => {
                    if Instant::now() >= deadline {
                        return incomplete(
                            frames,
                            bytes,
                            self.transport.wire_bytes_read() - before_wire,
                            Error::new(
                                "timeout",
                                "EOF observation exceeded original collection deadline",
                            ),
                        );
                    }
                    return ProtocolTransportEnd {
                        diagnostic: ProtocolTransportEndDiagnostic::CleanEof {
                            frames,
                            decoded_bytes: bytes,
                            wire_bytes: self.transport.wire_bytes_read() - before_wire,
                        },
                    };
                }
                TransportRead::Message(message) => message,
            };
            frames += 1;
            let size = match frame_bytes(&message) {
                Ok(size) => size,
                Err(error) => {
                    return incomplete(
                        frames,
                        bytes,
                        self.transport.wire_bytes_read() - before_wire,
                        error,
                    );
                }
            };
            bytes = bytes.saturating_add(size);
            if frames > MAX_TAIL_FRAMES || bytes > MAX_TAIL_BYTES {
                observer.observe_protocol(ProtocolObservation::Issue {
                    state: Some(state),
                    issue: ProtocolCaptureIssue::CaptureBound,
                });
                return incomplete(
                    frames,
                    bytes,
                    self.transport.wire_bytes_read() - before_wire,
                    Error::new(
                        "bounded_input",
                        "declared tail observation resource bound exceeded",
                    ),
                );
            }
            if let Err(error) = self.observe_tail_frame(message, state, observer) {
                return incomplete(
                    frames,
                    bytes,
                    self.transport.wire_bytes_read() - before_wire,
                    error,
                );
            }
            // Actual synchronous recording is part of collection, too.
            if Instant::now() >= deadline {
                return incomplete(
                    frames,
                    bytes,
                    self.transport.wire_bytes_read() - before_wire,
                    Error::new(
                        "timeout",
                        "tail recording exceeded original collection deadline",
                    ),
                );
            }
        }
    }
    fn observe_tail_frame(
        &mut self,
        message: RpcMessage,
        state: &mut SessionState,
        observer: &mut dyn ToolBroker,
    ) -> Result<()> {
        match message {
            RpcMessage::Request { id, method, params } => {
                self.server_request_count = self
                    .server_request_count
                    .checked_add(1)
                    .ok_or_else(|| Error::new("protocol_error", "server request count overflow"))?;
                observer.observe_protocol(ProtocolObservation::ServerRequest {
                    state: Some(state),
                    rpc_id: &id,
                    sequence: self.server_request_count,
                    method: &method,
                    params: &params,
                });
                observer.observe_protocol(ProtocolObservation::Issue {
                    state: Some(state),
                    issue: ProtocolCaptureIssue::LateUnservedRequest,
                });
                let bound = state.thread_id.as_deref().is_some_and(|thread| {
                    params.get("threadId").and_then(Value::as_str) == Some(thread)
                }) && params.get("turnId").and_then(Value::as_str).is_some_and(
                    |turn| {
                        state.completed_turns.contains_key(turn)
                            || state.turn_id.as_deref() == Some(turn)
                            || state.unresolved_turn.as_deref() == Some(turn)
                    },
                );
                if bound
                    && matches!(
                        method.as_str(),
                        "item/commandExecution/requestApproval"
                            | "item/fileChange/requestApproval"
                            | "execCommandApproval"
                            | "applyPatchApproval"
                            | "item/permissions/requestApproval"
                            | "item/tool/requestUserInput"
                            | "mcpServer/elicitation/request"
                    )
                    && !state.input_required
                {
                    state.input_required = true;
                    observer.observe_session(state)?;
                }
                // No tool call, no approval, no response write, no new effect.
            }
            RpcMessage::Notification { method, params } => {
                // Existing policy handling is deliberately policy-first: even
                // an otherwise invalid/new turn on the exact thread can carry
                // a genuine provider hold. Copy only sticky denial state.
                let mut candidate = state.clone();
                let exact_thread = state.thread_id.as_deref().is_some_and(|thread| {
                    params.get("threadId").and_then(Value::as_str) == Some(thread)
                });
                let parsed = if exact_thread {
                    candidate.notification(&method, &params)
                } else {
                    Err(Error::new(
                        "protocol_observation",
                        "tail notification is not bound to the actual thread",
                    ))
                };
                if candidate.intervention.is_some() && state.intervention.is_none() {
                    state.intervention = candidate.intervention.clone();
                    state.terminal = Some("blocked".into());
                    // Caller passes actual ScopedWorker, not a lease-first
                    // audit wrapper. Blocked checkpoint cannot finish proposals.
                    observer.observe_session(state)?;
                }
                let thread_matches =
                    params.get("threadId").and_then(Value::as_str) == state.thread_id.as_deref();
                let known_turn = params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
                    .is_some_and(|turn| state.completed_turns.contains_key(turn));
                let accepted = thread_matches && known_turn && parsed.is_ok();
                match method.as_str() {
                    "turn/completed" => {
                        observer.observe_protocol(ProtocolObservation::TerminalTurn {
                            state,
                            params: &params,
                            accepted,
                        })
                    }
                    "item/completed" => {
                        observer.observe_protocol(ProtocolObservation::CompletedItem {
                            state,
                            params: &params,
                            accepted,
                        })
                    }
                    "item/agentMessage/delta" => {
                        observer.observe_protocol(ProtocolObservation::AgentMessageDelta {
                            state,
                            params: &params,
                            accepted: false,
                        })
                    }
                    _ => observer.observe_protocol(ProtocolObservation::Issue {
                        state: Some(state),
                        issue: ProtocolCaptureIssue::UnsupportedTailFrame,
                    }),
                }
                if !accepted {
                    observer.observe_protocol(ProtocolObservation::Issue {
                        state: Some(state),
                        issue: ProtocolCaptureIssue::UnsupportedTailFrame,
                    });
                }
            }
            RpcMessage::Response { .. } | RpcMessage::Failure { .. } => {
                // A late response never resolves an uncertain native request.
                observer.observe_protocol(ProtocolObservation::Issue {
                    state: Some(state),
                    issue: ProtocolCaptureIssue::UnsupportedTailFrame,
                });
            }
        }
        Ok(())
    }
}
