use super::*;

/// Members by path with the fingerprints of their contents.
fn members(list: &[(&str, &[u8])]) -> BTreeMap<String, Fingerprint> {
    list.iter()
        .map(|(path, bytes)| ((*path).to_owned(), Fingerprint::of(bytes)))
        .collect()
}

/// Ids 1, 2, 3, ... after `start`, so tests know which id each new asset gets.
fn counter(start: u128) -> impl FnMut() -> Result<AssetId, AssetIndexError> {
    let mut next = start;
    move || {
        next += 1;
        Ok(AssetId::from_u128(next))
    }
}

fn id(value: u128) -> AssetId {
    AssetId::from_u128(value)
}

fn asset(value: u128, path: &str) -> Asset {
    Asset {
        id: id(value),
        path: path.to_owned(),
    }
}

/// An index of `list`, ids 1, 2, 3, ... in path order.
fn indexed(list: &[(&str, &[u8])]) -> AssetIndex {
    let mut index = AssetIndex::new();
    index.update(&members(list), counter(0)).unwrap();
    index
}

#[test]
fn new_members_get_new_ids_in_path_order_and_metadata_is_skipped() {
    let mut index = AssetIndex::new();
    let report = index
        .update(
            &members(&[
                ("metallic.toml", b"[project]"),
                ("assets.json", b"{}"),
                ("Scripts/Hello.luau", b"print('hi')"),
                ("Assets/grass.png", b"png"),
                // The editor's own data (#603) is not an asset either.
                (".studio/explorer.json", b"{}"),
            ]),
            counter(0),
        )
        .unwrap();
    assert_eq!(
        report,
        IndexReport {
            added: vec![asset(1, "Assets/grass.png"), asset(2, "Scripts/Hello.luau")],
            ..IndexReport::default()
        }
    );
    assert_eq!(index.len(), 2);
    assert_eq!(index.id("Assets/grass.png"), Some(id(1)));
    assert_eq!(index.path(id(2)), Some("Scripts/Hello.luau"));
    assert_eq!(index.id("metallic.toml"), None);
    assert_eq!(index.fingerprint("Assets/grass.png"), Some(Fingerprint::of(b"png")));
}

#[test]
fn a_moved_or_renamed_member_keeps_its_id() {
    let mut index = indexed(&[("Assets/a.png", b"aaa"), ("Assets/b.png", b"bbb")]);
    let report = index
        .update(
            &members(&[("Assets/Textures/a.png", b"aaa"), ("Assets/c.png", b"bbb")]),
            counter(100),
        )
        .unwrap();
    assert_eq!(
        report.moved,
        [
            AssetMove {
                id: id(1),
                from: "Assets/a.png".into(),
                to: "Assets/Textures/a.png".into()
            },
            AssetMove {
                id: id(2),
                from: "Assets/b.png".into(),
                to: "Assets/c.png".into()
            },
        ]
    );
    assert!(report.added.is_empty() && report.missing.is_empty());
    assert_eq!(index.id("Assets/Textures/a.png"), Some(id(1)));
    assert_eq!(index.id("Assets/c.png"), Some(id(2)));
    assert_eq!(index.id("Assets/a.png"), None);
    assert_eq!(index.len(), 2);
}

#[test]
fn a_member_changed_in_place_keeps_its_id_and_its_new_fingerprint_is_kept() {
    let mut index = indexed(&[("Scripts/a.luau", b"one")]);
    let report = index
        .update(&members(&[("Scripts/a.luau", b"two")]), counter(100))
        .unwrap();
    assert_eq!(report, IndexReport::default());
    assert_eq!(index.id("Scripts/a.luau"), Some(id(1)));
    // So a later move of the changed contents is still found.
    let report = index
        .update(&members(&[("Scripts/b.luau", b"two")]), counter(100))
        .unwrap();
    assert_eq!(report.moved.len(), 1);
    assert_eq!(index.id("Scripts/b.luau"), Some(id(1)));
}

#[test]
fn a_deleted_member_is_reported_missing_until_it_comes_back_or_is_forgotten() {
    let mut index = indexed(&[("Assets/a.png", b"aaa"), ("Assets/b.png", b"bbb")]);
    let report = index
        .update(&members(&[("Assets/b.png", b"bbb")]), counter(100))
        .unwrap();
    assert_eq!(report.missing, [asset(1, "Assets/a.png")]);
    assert_eq!(
        index.id("Assets/a.png"),
        Some(id(1)),
        "a missing asset stays in the index"
    );

    // Still missing on the next run.
    let report = index
        .update(&members(&[("Assets/b.png", b"bbb")]), counter(100))
        .unwrap();
    assert_eq!(report.missing, [asset(1, "Assets/a.png")]);

    // Back, somewhere else: it is the same asset.
    let report = index
        .update(
            &members(&[("Assets/b.png", b"bbb"), ("Assets/Restored/a.png", b"aaa")]),
            counter(100),
        )
        .unwrap();
    assert!(report.missing.is_empty());
    assert_eq!(index.id("Assets/Restored/a.png"), Some(id(1)));

    assert_eq!(index.forget(id(1)), Some("Assets/Restored/a.png".to_owned()));
    assert_eq!(index.path(id(1)), None);
    assert_eq!(index.forget(id(1)), None);
}

#[test]
fn a_member_moved_and_changed_at_once_is_missing_plus_a_new_asset() {
    let mut index = indexed(&[("Assets/a.png", b"aaa")]);
    let report = index
        .update(&members(&[("Assets/b.png", b"changed")]), counter(100))
        .unwrap();
    assert_eq!(report.added, [asset(101, "Assets/b.png")]);
    assert_eq!(report.missing, [asset(1, "Assets/a.png")]);
}

#[test]
fn empty_members_are_never_matched_as_moves() {
    let mut index = indexed(&[("Scripts/a.luau", b"")]);
    let report = index
        .update(&members(&[("Scripts/b.luau", b"")]), counter(100))
        .unwrap();
    assert_eq!(report.added, [asset(101, "Scripts/b.luau")]);
    assert_eq!(report.missing, [asset(1, "Scripts/a.luau")]);
}

#[test]
fn identical_members_moved_together_pair_up_in_path_order() {
    let mut index = indexed(&[("Old/x.png", b"same"), ("Old/y.png", b"same")]);
    index
        .update(
            &members(&[("New/x.png", b"same"), ("New/y.png", b"same")]),
            counter(100),
        )
        .unwrap();
    assert_eq!(index.id("New/x.png"), Some(id(1)));
    assert_eq!(index.id("New/y.png"), Some(id(2)));
}

#[test]
fn a_new_id_already_in_use_is_refused_and_the_index_is_unchanged() {
    let mut index = indexed(&[("Assets/a.png", b"aaa")]);
    let before = index.clone();
    let error = index
        .update(
            &members(&[("Assets/a.png", b"aaa"), ("Assets/b.png", b"bbb")]),
            counter(0),
        )
        .unwrap_err();
    assert!(matches!(error, AssetIndexError::IdInUse(used) if used == id(1)));
    assert_eq!(index, before);
}

#[test]
fn the_json_is_sorted_one_asset_per_line_and_round_trips_byte_for_byte() {
    let index = indexed(&[("Scripts/Hello \"there\".luau", b"print()"), ("Assets/a.png", b"aaa")]);
    let json = index.to_json();
    let aaa = crc32fast::hash(b"aaa");
    let hello = crc32fast::hash(b"print()");
    assert_eq!(
        json,
        format!(
            "{{\n  \"format\": 1,\n  \"assets\": {{\n    \
             \"Assets/a.png\": {{\"id\": \"00000000000000000000000000000001\", \"size\": 3, \"crc32\": {aaa}}},\n    \
             \"Scripts/Hello \\\"there\\\".luau\": {{\"id\": \"00000000000000000000000000000002\", \"size\": 7, \
             \"crc32\": {hello}}}\n  }}\n}}\n"
        )
    );
    let read = AssetIndex::from_bytes(json.as_bytes()).unwrap();
    assert_eq!(read, index);
    assert_eq!(read.to_json(), json);

    let empty = AssetIndex::new().to_json();
    assert_eq!(empty, "{\n  \"format\": 1,\n  \"assets\": {}\n}\n");
    assert_eq!(AssetIndex::from_bytes(empty.as_bytes()).unwrap(), AssetIndex::new());
}

#[test]
fn a_bad_index_is_refused() {
    let entry = |path: &str, id: &str| format!(r#""{path}": {{"id": "{id}", "size": 1, "crc32": 2}}"#);
    let file = |entries: &[String]| format!(r#"{{"format": 1, "assets": {{{}}}}}"#, entries.join(", "));
    let one = "00000000000000000000000000000001";
    let refused = |text: String| AssetIndex::from_bytes(text.as_bytes()).unwrap_err();

    assert!(matches!(refused("not json".into()), AssetIndexError::Json(_)));
    assert!(matches!(
        refused(r#"{"format": 2, "assets": {"a": 7}}"#.into()),
        AssetIndexError::UnsupportedFormat { found: 2 }
    ));
    assert!(matches!(
        refused(file(&[entry("../a.png", one)])),
        AssetIndexError::BadPath { .. }
    ));
    assert!(matches!(
        refused(file(&[entry("metallic.toml", one)])),
        AssetIndexError::BadPath { .. }
    ));
    assert!(matches!(
        refused(file(&[entry("a.png", "0000000000000000000000000000000A")])),
        AssetIndexError::BadId { .. }
    ));
    assert!(matches!(
        refused(file(&[entry("a.png", one), entry("b.png", one)])),
        AssetIndexError::DuplicateId { .. }
    ));
}

#[test]
fn ids_have_one_spelling_and_random_ones_differ() {
    let id = AssetId::from_u128(0xabc);
    assert_eq!(id.to_string(), "00000000000000000000000000000abc");
    assert_eq!("00000000000000000000000000000abc".parse(), Ok(id));
    assert_eq!(
        "00000000000000000000000000000ABC".parse::<AssetId>(),
        Err(ParseAssetIdError)
    );
    assert_eq!("abc".parse::<AssetId>(), Err(ParseAssetIdError));
    assert_eq!(
        "+0000000000000000000000000000abc".parse::<AssetId>(),
        Err(ParseAssetIdError)
    );
    assert_ne!(AssetId::random().unwrap(), AssetId::random().unwrap());
}
