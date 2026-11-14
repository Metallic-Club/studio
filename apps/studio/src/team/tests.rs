use std::path::Path;

use super::*;

#[test]
fn a_copy_s_path_is_safe_whatever_the_project_is_called() {
    let cache = Path::new("cache");
    let path = copy_path(cache, ProjectId(1), "../../Evil: Name?");
    assert!(path.starts_with(cache));
    assert_eq!(path.file_name().unwrap(), "______Evil_ Name_.mtproj");
    assert_eq!(
        copy_path(cache, ProjectId(1), "   ").file_name().unwrap(),
        "Project.mtproj"
    );
}

#[test]
fn ports_are_numbers_from_1_to_65535() {
    assert_eq!(parse_port(" 7350 "), Ok(7350));
    assert!(parse_port("0").is_err());
    assert!(parse_port("70000").is_err());
    assert!(parse_port("abc").is_err());
}

#[test]
fn joining_nothing_ends_with_a_reason_and_no_session() {
    let mut team = TeamCreate::in_memory();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    team.join(&format!("127.0.0.1:{port}"), Identity::new("Bob", [1, 2, 3]), None)
        .unwrap();
    assert!(team.is_active());
    let start = Instant::now();
    while team.is_active() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(15),
            "the join never failed"
        );
        team.update(None, None);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let note = team.note.clone().expect("a reason");
    assert!(note.error);
    assert!(note.text.contains("127.0.0.1"), "{}", note.text);
    assert!(
        team.join("", Identity::new("Bob", [1, 2, 3]), None).is_err(),
        "an empty address"
    );
}
