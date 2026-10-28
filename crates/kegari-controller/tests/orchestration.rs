use kegari_controller::orchestration::*;
use kegari_types::control::Lease;
use kegari_v0::control::Intent;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[tokio::test]
async fn partial_control_frame_survives_cancelled_read_and_eof_is_not_acknowledgement() {
    let (mut input, output) = tokio::io::duplex(1024);
    let mut reader = FrameReader::new(output);
    let frame = serde_json::to_vec(&HostFrame::State {
        desired: Intent::Paused,
        control_generation: 9,
    })
    .unwrap();
    input.write_all(&frame[..10]).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(5),
            reader.next::<HostFrame>()
        )
        .await
        .is_err()
    );
    input.write_all(&frame[10..]).await.unwrap();
    input.write_all(b"\n").await.unwrap();
    assert!(matches!(
        reader.next::<HostFrame>().await.unwrap(),
        HostFrame::State {
            desired: Intent::Paused,
            control_generation: 9
        }
    ));
    input.write_all(b"{\"kind\":\"state\"").await.unwrap();
    drop(input);
    assert_eq!(
        reader.next::<HostFrame>().await.err().unwrap().code,
        "control_unavailable"
    );
}

#[tokio::test]
async fn process_channel_is_closed_and_size_bounded() {
    for bytes in [
        b"{\"kind\":\"shell\",\"command\":\"private-sentinel\"}\n".to_vec(),
        vec![b'x'; CHANNEL_BOUND + 1],
    ] {
        let (mut input, output) = tokio::io::duplex(CHANNEL_BOUND + 64);
        input.write_all(&bytes).await.unwrap();
        drop(input);
        let error = FrameReader::new(output)
            .next::<HostFrame>()
            .await
            .err()
            .unwrap();
        assert!(["body_too_large", "incompatible_version"].contains(&error.code.as_str()));
        assert!(!format!("{error:?}").contains("private-sentinel"));
    }
}

#[test]
fn stale_or_contradictory_state_cannot_restart_paused_or_stopped_cycles() {
    let lease = Lease {
        runtime_id: Uuid::new_v4(),
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    let mut state = CycleState::new(lease.clone());
    assert!(state.poll().unwrap().is_none());
    state.update(Intent::Running, 1).unwrap();
    assert!(
        matches!(state.poll().unwrap(),Some(ChildFrame::Poll{lease:observed,sequence:1}) if observed==lease)
    );
    state.update(Intent::Paused, 2).unwrap();
    assert!(state.update(Intent::Running, 1).is_err());
    assert!(state.update(Intent::Running, 2).is_err());
    assert!(state.poll().unwrap().is_none());
    state.update(Intent::Stopped, 3).unwrap();
    assert!(state.poll().unwrap().is_none());
    state.update(Intent::Running, 4).unwrap();
    assert!(matches!(
        state.poll().unwrap(),
        Some(ChildFrame::Poll { sequence: 2, .. })
    ));
}
