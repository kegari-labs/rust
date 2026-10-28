//! Separate orchestration process with an inherited, bounded control channel.
//! The child owns runtime.lock; the host owns admission and all provider effects.
//! No user-controlled socket/header can construct this process handshake.
use crate::{
    control::{ControlHost, Intent, Lease},
    platform::{
        BinaryIdentity, HostIdentity, OwnedChild, ProcessIdentity, Registration, RuntimeLock,
    },
};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::unix::pipe,
    time::{Instant, MissedTickBehavior, timeout},
};

pub const CHANNEL_BOUND: usize = 64 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HOST_SILENCE: Duration = Duration::from_secs(20);
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostFrame {
    Welcome {
        lease: Lease,
        poll_ms: u64,
    },
    State {
        desired: Intent,
        control_generation: u64,
    },
    Heartbeat,
    Exit,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChildFrame {
    Ready {
        api_version: u32,
        runtime_build: String,
        lease: Lease,
        process: ProcessIdentity,
    },
    Poll {
        lease: Lease,
        sequence: u64,
    },
    Exiting {
        lease: Lease,
        last_sequence: u64,
    },
}

pub struct CycleState {
    lease: Lease,
    desired: Intent,
    generation: Option<u64>,
    sequence: u64,
}
impl CycleState {
    pub fn new(lease: Lease) -> Self {
        Self {
            lease,
            desired: Intent::Stopped,
            generation: None,
            sequence: 0,
        }
    }
    pub fn update(&mut self, desired: Intent, generation: u64) -> Result<()> {
        if self.generation.is_some_and(|prior| {
            generation < prior || (generation == prior && desired != self.desired)
        }) {
            return Err(Error::new(
                "stale_control",
                "host control state is stale or contradictory",
            ));
        }
        self.desired = desired;
        self.generation = Some(generation);
        Ok(())
    }
    pub fn may_poll(&self) -> bool {
        self.generation.is_some() && self.desired == Intent::Running
    }
    pub fn poll(&mut self) -> Result<Option<ChildFrame>> {
        if !self.may_poll() {
            return Ok(None);
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| Error::new("ownership_conflict", "cycle sequence exhausted"))?;
        Ok(Some(ChildFrame::Poll {
            lease: self.lease.clone(),
            sequence: self.sequence,
        }))
    }
}

pub struct FrameReader<R> {
    reader: R,
    partial: Vec<u8>,
}
impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            partial: Vec::new(),
        }
    }
    /// Partial input survives cancellation by a timer/select branch. EOF never
    /// turns a truncated frame into an acknowledged state transition.
    pub async fn next<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut byte = [0u8; 1];
        loop {
            let n = self.reader.read(&mut byte).await.map_err(|_| {
                Error::new("control_unavailable", "orchestration channel read failed")
            })?;
            if n == 0 {
                return Err(Error::new(
                    "control_unavailable",
                    "orchestration channel closed; preserve unfinished operations",
                ));
            }
            if byte[0] == b'\n' {
                let frame = std::mem::take(&mut self.partial);
                return serde_json::from_slice(&frame).map_err(|_| {
                    Error::new("incompatible_version", "invalid closed orchestration frame")
                });
            }
            if self.partial.len() >= CHANNEL_BOUND {
                return Err(Error::new(
                    "body_too_large",
                    "orchestration frame exceeds bound",
                ));
            }
            self.partial.push(byte[0]);
        }
    }
}
pub async fn send_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| Error::new("invalid_input", "cannot encode orchestration frame"))?;
    if bytes.len() > CHANNEL_BOUND {
        return Err(Error::new(
            "body_too_large",
            "orchestration frame exceeds bound",
        ));
    }
    bytes.push(b'\n');
    timeout(HANDSHAKE_TIMEOUT, async {
        writer.write_all(&bytes).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| Error::new("timeout", "orchestration channel write timed out"))?
    .map_err(|_| Error::new("control_unavailable", "orchestration channel write failed"))
}

pub struct RunningOrchestration {
    child: OwnedChild,
    writer: pipe::Sender,
    reader: FrameReader<pipe::Receiver>,
    lease: Lease,
    sequence: u64,
    activated: bool,
    control: Arc<ControlHost>,
    registration: Arc<Registration>,
}
impl RunningOrchestration {
    pub async fn spawn(
        registration: Arc<Registration>,
        control: Arc<ControlHost>,
        host: &HostIdentity,
        binary: &BinaryIdentity,
        poll_ms: u64,
    ) -> Result<Self> {
        if control.snapshot()?.intent.desired != Intent::Running {
            return Err(Error::new(
                "requires_start",
                "saved intent forbids orchestration startup",
            ));
        }
        if poll_ms == 0
            || Instant::now()
                .checked_add(Duration::from_millis(poll_ms))
                .is_none()
        {
            return Err(Error::invalid("orchestration poll interval is unsupported"));
        }
        let lease = registration.prepare_owner()?;
        let mut child = OwnedChild::spawn(binary, &["__orchestrate".into()], &host.home)?;
        let (input, output, errors) = child.take_io()?;
        let mut writer = pipe::Sender::from_owned_fd(input.into())
            .map_err(|e| Error::io("open inherited orchestration input", e))?;
        let output = pipe::Receiver::from_owned_fd(output.into())
            .map_err(|e| Error::io("open inherited orchestration output", e))?;
        let mut errors = pipe::Receiver::from_owned_fd(errors.into())
            .map_err(|e| Error::io("open inherited orchestration diagnostics", e))?;
        // Diagnostic text is deliberately not copied to logs/status. The child
        // protocol reports typed safe errors; consuming stderr prevents deadlock.
        tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while let Ok(n) = errors.read(&mut buffer).await {
                if n == 0 {
                    break;
                }
            }
        });
        let mut reader = FrameReader::new(output);
        send_frame(
            &mut writer,
            &HostFrame::Welcome {
                lease: lease.clone(),
                poll_ms,
            },
        )
        .await?;
        let ready = timeout(HANDSHAKE_TIMEOUT, reader.next::<ChildFrame>())
            .await
            .map_err(|_| Error::new("timeout", "orchestration startup did not acknowledge"))??;
        match ready {
            ChildFrame::Ready {
                api_version: 1,
                runtime_build,
                lease: observed,
                process,
            } if runtime_build == crate::BUILD_ID
                && observed == lease
                && &process == child.identity() =>
            {
                registration.record_orchestration(lease.clone(), process)?
            }
            _ => {
                return Err(Error::new(
                    "identity_mismatch",
                    "orchestration handshake differs from the owned child/build/lease",
                ));
            }
        }
        Ok(Self {
            child,
            writer,
            reader,
            lease,
            sequence: 0,
            activated: false,
            control,
            registration,
        })
    }
    pub fn lease(&self) -> &Lease {
        &self.lease
    }
    pub fn process(&self) -> &ProcessIdentity {
        self.child.identity()
    }
    /// The concrete engine must reconstruct provider facts, preserved work,
    /// attempts and uncertain holds before this opens autonomous admission.
    pub async fn activate(&mut self, recovery: impl Future<Output = Result<()>>) -> Result<()> {
        tokio::pin!(recovery);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = &mut recovery => {result?;break;},
                _ = heartbeat.tick() => send_frame(&mut self.writer, &HostFrame::Heartbeat).await?,
            }
        }
        self.registration.verify_active_owner(&self.lease)?;
        self.control.attach_reconciled(self.lease.clone())?;
        self.activated = true;
        self.refresh_intent().await
    }
    pub async fn refresh_intent(&mut self) -> Result<()> {
        let snapshot = self.control.snapshot()?;
        send_frame(
            &mut self.writer,
            &HostFrame::State {
                desired: snapshot.intent.desired,
                control_generation: snapshot.intent.generation,
            },
        )
        .await
    }
    /// A cycle is a scheduling opportunity, never an admission permit. Every
    /// operation still enters ControlHost::admit and the current mutation gate.
    pub async fn next_cycle(&mut self) -> Result<u64> {
        if !self.activated {
            return Err(Error::new(
                "reconciliation_required",
                "orchestration has not completed recovery",
            ));
        }
        match self.reader.next::<ChildFrame>().await? {
            ChildFrame::Poll { lease, sequence }
                if lease == self.lease
                    && sequence
                        == self.sequence.checked_add(1).ok_or_else(|| {
                            Error::new("ownership_conflict", "cycle sequence exhausted")
                        })? =>
            {
                self.sequence = sequence;
                let snapshot = self.control.snapshot()?;
                if !snapshot.admission_open || snapshot.intent.desired != Intent::Running {
                    return Err(Error::new(
                        "admission_closed",
                        "poll arrived after current intent closed admission",
                    ));
                }
                Ok(sequence)
            }
            ChildFrame::Exiting { lease, .. } if lease == self.lease => Err(Error::new(
                "control_unavailable",
                "orchestration is exiting",
            )),
            _ => Err(Error::new(
                "ownership_conflict",
                "orchestration frame has a stale lease or repeated sequence",
            )),
        }
    }
    pub fn exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }
    /// Retire only the scheduling process after its owner has been durably
    /// suspended. Worker cancellation/recovery remains the supervisor's job.
    pub async fn retire_failed(&mut self) -> Result<()> {
        let snapshot = self.control.snapshot()?;
        if snapshot.admission_open
            || snapshot.reconciled
            || snapshot
                .active_operations
                .iter()
                .any(|p| !snapshot.fenced_operations.contains(&p.operation_id))
        {
            return Err(Error::new(
                "access_denied",
                "failed owner must be suspended before retiring its process",
            ));
        }
        if !self.exited()? {
            send_frame(&mut self.writer, &HostFrame::Exit).await?;
        }
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        while !self.exited()? {
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "stop_in_progress",
                    "failed scheduling process exit is unconfirmed",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if self.activated && self.control.snapshot()?.orchestration_alive {
            self.control.orchestration_exited(&self.lease)?;
            self.activated = false;
        }
        Ok(())
    }
    pub async fn shutdown(&mut self, force: bool) -> Result<()> {
        let snapshot = self.control.snapshot()?;
        if snapshot.intent.desired != Intent::Stopped
            || (!force && !snapshot.active_operations.is_empty())
            || (force
                && snapshot
                    .active_operations
                    .iter()
                    .any(|permit| !snapshot.fenced_operations.contains(&permit.operation_id)))
        {
            return Err(Error::new(
                "stop_in_progress",
                "shutdown requires durable STOPPED and drained or force-fenced work",
            ));
        }
        if !self.exited()? {
            if force {
                self.child.signal_group(true)?;
            } else {
                send_frame(&mut self.writer, &HostFrame::Exit).await?;
            }
        }
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        while !self.exited()? {
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "stop_in_progress",
                    "owned orchestration shutdown is not confirmed",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if self.activated {
            self.control.orchestration_exited(&self.lease)?;
            self.activated = false;
        }
        Ok(())
    }
}

/// Entry point for the trusted installed binary's internal orchestration mode.
/// It starts no provider clients or workers and performs no independent writes.
/// Pipe closure/host silence releases runtime.lock and exits conservatively.
pub async fn run_child() -> Result<()> {
    let host = HostIdentity::discover()?;
    let root = PrivateDir::open(&host.state_root())?;
    let mut reader = FrameReader::new(tokio::io::stdin());
    let mut writer = tokio::io::stdout();
    let welcome = timeout(HANDSHAKE_TIMEOUT, reader.next::<HostFrame>())
        .await
        .map_err(|_| Error::new("control_unavailable", "no inherited control-host handshake"))??;
    let HostFrame::Welcome { lease, poll_ms } = welcome else {
        return Err(Error::new(
            "ownership_conflict",
            "orchestration must begin with its reserved lease",
        ));
    };
    if poll_ms == 0
        || Instant::now()
            .checked_add(Duration::from_millis(poll_ms))
            .is_none()
    {
        return Err(Error::invalid("invalid orchestration poll interval"));
    }
    let _lock = RuntimeLock::acquire(&root, lease.clone())?;
    send_frame(
        &mut writer,
        &ChildFrame::Ready {
            api_version: 1,
            runtime_build: crate::BUILD_ID.into(),
            lease: lease.clone(),
            process: ProcessIdentity::current()?,
        },
    )
    .await?;
    let mut interval = tokio::time::interval(Duration::from_millis(poll_ms));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut state = CycleState::new(lease.clone());
    let mut last_host = Instant::now();
    loop {
        tokio::select! {
            frame=reader.next::<HostFrame>()=>match frame? {
                HostFrame::State{desired:next,control_generation}=>{
                    state.update(next,control_generation)?;last_host=Instant::now();
                }
                HostFrame::Heartbeat=>{last_host=Instant::now();}
                HostFrame::Exit=>{send_frame(&mut writer,&ChildFrame::Exiting{lease,last_sequence:state.sequence}).await?;return Ok(());}
                HostFrame::Welcome{..}=>return Err(Error::new("ownership_conflict","startup lease cannot be replaced on a live channel")),
            },
            _=interval.tick(),if state.may_poll()=>{
                if let Some(poll)=state.poll()? {send_frame(&mut writer,&poll).await?;}
            },
            _=tokio::time::sleep_until(last_host+HOST_SILENCE)=>return Err(Error::new("control_unavailable","control host heartbeat expired")),
        }
    }
}
