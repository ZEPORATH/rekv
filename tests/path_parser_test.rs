use rekv::path_parser::{QueryPath, Segment, Selector};

#[test]
fn parses_plain_path_and_root() {
    assert!(QueryPath::parse("/").unwrap().is_root());
    assert_eq!(
        QueryPath::parse("/platform_manager/io_devices")
            .unwrap()
            .segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Key("io_devices".to_string())
        ]
    );
}

#[test]
fn parses_id_and_index_selectors_with_optional_whitespace() {
    let compact = QueryPath::parse("/platform_manager/io_devices[id=ECU0]/baud_rate").unwrap();
    let spaced = QueryPath::parse("/platform_manager/io_devices[id = ECU0 ]/baud_rate").unwrap();
    assert_eq!(compact, spaced);
    assert_eq!(
        compact.segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Key("io_devices".to_string()),
            Segment::Selector(Selector::Id("ECU0".to_string())),
            Segment::Key("baud_rate".to_string())
        ]
    );
    assert_eq!(
        QueryPath::parse("/platform_manager/io_devices[idx = 1]/baud_rate")
            .unwrap()
            .segments[2],
        Segment::Selector(Selector::Index(1))
    );
}

#[test]
fn parses_selected_object_wildcard() {
    assert_eq!(
        QueryPath::parse("/platform_manager/io_devices[id = ECU0]/*")
            .unwrap()
            .segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Key("io_devices".to_string()),
            Segment::Selector(Selector::Id("ECU0".to_string())),
            Segment::Wildcard
        ]
    );
}

#[test]
fn rejects_legacy_and_invalid_selector_syntax() {
    for path in [
        "/platform_manager/io_devices#ECU0/baud_rate",
        "/platform_manager/io_devices[type = ECU0]",
        "/platform_manager/io_devices[id ECU0]",
        "/platform_manager/io_devices[idx = -1]",
        "/platform_manager/io_devices[id = ]",
        "/platform_manager/io_devices[id=ECU0][idx=0]",
        "/**/baud_rate",
    ] {
        assert!(
            QueryPath::parse(path).is_err(),
            "accepted invalid path: {}",
            path
        );
    }
}
