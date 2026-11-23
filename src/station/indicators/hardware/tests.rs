use super::*;

fn fixture() -> (tempfile::TempDir, Hardware) {
    let dir = tempfile::tempdir().unwrap();
    let ring = dir.path().join("fb0");
    fs::write(&ring, [3; RING_PIXELS * 3]).unwrap();
    let segments = (0..SEGMENTS)
        .map(|index| {
            let path = dir.path().join(index.to_string());
            fs::write(&path, b"64\n").unwrap();
            Segment { path, max: 255 }
        })
        .collect::<Vec<_>>();
    (dir, Hardware { ring, segments })
}

fn write_clock(segments: &[Segment], frame: &[u8]) {
    assert_eq!(segments.len(), frame.len());
    for (segment, &value) in segments.iter().zip(frame) {
        set_brightness(&segment.path, value).unwrap();
    }
}

#[test]
fn restoring_indicators_preserves_new_writes_from_the_station() {
    let (_dir, hardware) = fixture();
    let frame = Frame {
        ring: [12; RING_PIXELS * 3],
        clock: [128; SEGMENTS],
    };
    {
        let mut lights = hardware.begin();
        lights.show(&frame);
        assert_eq!(fs::read(&hardware.ring).unwrap(), frame.ring);
        assert_eq!(brightness(&hardware.segments[0].path), Some(32));
    }
    assert_eq!(fs::read(&hardware.ring).unwrap(), [3; RING_PIXELS * 3]);
    assert_eq!(brightness(&hardware.segments[0].path), Some(64));

    let mut lights = hardware.begin();
    lights.show(&frame);
    let native_ring = [24; RING_PIXELS * 3];
    fs::write(&hardware.ring, native_ring).unwrap();
    let native_display = [20; SEGMENTS];
    write_clock(&hardware.segments, &native_display);
    lights.show(&frame);
    assert_eq!(fs::read(&hardware.ring).unwrap(), native_ring);
    assert_eq!(
        read_clock(hardware.segments.as_slice().try_into().unwrap()),
        Some(native_display)
    );
}

#[test]
fn clock_redraw_waits_for_a_complete_frame_before_resuming_the_face() {
    let (_dir, hardware) = fixture();
    let segments: &[_; SEGMENTS] = hardware.segments.as_slice().try_into().unwrap();
    let mut clock = Clock::open(segments).unwrap();
    let mut eyes = [0; SEGMENTS];
    eyes[0] = 255;
    eyes[21] = 255;
    let now = Instant::now();
    assert!(clock.show(&eyes, now));
    let face = read_clock(segments).unwrap();

    let mut latest_time = [0; SEGMENTS];
    latest_time[1] = 64;
    latest_time[9] = 79;
    let redraw_started = now + CLOCK_READBACK * 2;
    write_clock(&segments[..7], &latest_time[..7]);
    assert!(clock.show(&eyes, redraw_started));
    let partial = read_clock(segments);
    assert_ne!(partial, Some(face));
    assert!(clock.show(&eyes, redraw_started + CLOCK_SETTLE / 2));
    assert_eq!(read_clock(segments), partial);

    let redraw_finished = redraw_started + CLOCK_SETTLE;
    write_clock(&segments[7..], &latest_time[7..]);
    assert!(clock.show(&eyes, redraw_finished));
    assert_eq!(read_clock(segments), Some(latest_time));
    assert!(clock.show(&eyes, redraw_finished + CLOCK_SETTLE));
    assert_eq!(read_clock(segments), Some(face));

    drop(clock);
    assert_eq!(read_clock(segments), Some(latest_time));
}

#[test]
fn finishing_during_a_native_redraw_leaves_the_new_clock_frame_intact() {
    let (_dir, hardware) = fixture();
    let segments: &[_; SEGMENTS] = hardware.segments.as_slice().try_into().unwrap();
    let mut clock = Clock::open(segments).unwrap();
    let mut eyes = [0; SEGMENTS];
    eyes[0] = 255;
    assert!(clock.show(&eyes, Instant::now()));

    let mut native = [0; SEGMENTS];
    native[1] = 64;
    native[21] = 64;
    write_clock(segments, &native);
    drop(clock);

    assert_eq!(read_clock(segments), Some(native));
}

#[test]
fn a_native_ring_clear_does_not_end_the_active_display() {
    let (_dir, hardware) = fixture();
    let frame = Frame {
        ring: [12; RING_PIXELS * 3],
        clock: [128; SEGMENTS],
    };
    let mut lights = hardware.begin();
    lights.show(&frame);
    fs::write(&hardware.ring, [0; RING_PIXELS * 3]).unwrap();
    lights.show(&frame);
    assert_eq!(fs::read(&hardware.ring).unwrap(), frame.ring);
    assert_eq!(brightness(&hardware.segments[0].path), Some(32));

    drop(lights);
    assert_eq!(fs::read(&hardware.ring).unwrap(), [0; RING_PIXELS * 3]);
    assert_eq!(brightness(&hardware.segments[0].path), Some(64));
}
