use omnimotion::machine::log::{LogBuffer, MAX_LINE};

fn text<const N: usize>(log: &LogBuffer<N>) -> String {
    log.text(&mut [0; N]).to_owned()
}

#[test]
fn empty_buffer_has_no_text() {
    assert_eq!(text(&LogBuffer::<16>::new()), "");
}

#[test]
fn keeps_lines_oldest_first() {
    let mut log = LogBuffer::<64>::new();
    log.push_line("one");
    log.push_line("two");
    assert_eq!(text(&log), "one\ntwo\n");
}

#[test]
fn overwrites_the_oldest_bytes_when_full() {
    let mut log = LogBuffer::<16>::new();
    log.push_line("aaaaaaaa");
    log.push_line("bbbbbbbb");
    log.push_line("cc");
    let text = text(&log);
    assert!(text.len() <= 16, "{text:?}");
    assert!(text.ends_with("bbbbbbbb\ncc\n"), "{text:?}");
}

#[test]
fn cuts_long_lines_at_a_character_boundary() {
    let mut log = LogBuffer::<512>::new();
    log.push_line(&"é".repeat(200));
    let text = text(&log);
    assert!(text.ends_with('\n'));
    assert!(text.len() <= MAX_LINE + 1);
    assert!(text.trim_end().chars().all(|c| c == 'é'));
}

#[test]
fn an_overwritten_character_is_dropped_whole() {
    let mut log = LogBuffer::<8>::new();
    log.push_line("éééé");
    let text = text(&log);
    assert!(text.chars().all(|c| c == 'é' || c == '\n'), "{text:?}");
    assert!(text.ends_with("é\n"));
}
