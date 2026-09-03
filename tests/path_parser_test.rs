use rekv::path_parser::{Predicate, PredicateOp, PredicateValue, QueryPath, Segment};

#[test]
fn test_parse_empty_and_root() {
    assert_eq!(QueryPath::parse("").unwrap(), QueryPath { segments: vec![] });
    assert_eq!(QueryPath::parse("/").unwrap(), QueryPath { segments: vec![] });
    assert_eq!(QueryPath::parse("   /   ").unwrap(), QueryPath { segments: vec![] });
}

#[test]
fn test_parse_simple_keys() {
    let q = QueryPath::parse("/platform_manager/grpc_port").unwrap();
    assert_eq!(
        q.segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Key("grpc_port".to_string())
        ]
    );
}

#[test]
fn test_parse_numeric_indices() {
    let q1 = QueryPath::parse("/peripherals/0/pin").unwrap();
    assert_eq!(
        q1.segments,
        vec![
            Segment::Key("peripherals".to_string()),
            Segment::Index(0),
            Segment::Key("pin".to_string())
        ]
    );

    let q2 = QueryPath::parse("/peripherals[0]/pin").unwrap();
    assert_eq!(
        q2.segments,
        vec![
            Segment::Key("peripherals".to_string()),
            Segment::Index(0),
            Segment::Key("pin".to_string())
        ]
    );
}

#[test]
fn test_parse_id_shorthand() {
    let q = QueryPath::parse("/platform_manager/peripherals#REED_UP/pin").unwrap();
    assert_eq!(
        q.segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Key("peripherals".to_string()),
            Segment::Id("REED_UP".to_string()),
            Segment::Key("pin".to_string())
        ]
    );
}

#[test]
fn test_parse_predicate_queries() {
    // String with quotes
    let q1 = QueryPath::parse("/peripherals[type=\"reed\"]/pin").unwrap();
    assert_eq!(
        q1.segments,
        vec![
            Segment::Key("peripherals".to_string()),
            Segment::Predicates(vec![Predicate {
                key: "type".to_string(),
                op: PredicateOp::Eq,
                value: PredicateValue::String("reed".to_string()),
            }]),
            Segment::Key("pin".to_string())
        ]
    );

    // Numeric comparison >=
    let q2 = QueryPath::parse("/peripherals[pin>=20]").unwrap();
    assert_eq!(
        q2.segments,
        vec![
            Segment::Key("peripherals".to_string()),
            Segment::Predicates(vec![Predicate {
                key: "pin".to_string(),
                op: PredicateOp::Gte,
                value: PredicateValue::Number(20.0),
            }])
        ]
    );

    // Multiple chained predicates: [type="relay"][default=0]
    let q3 = QueryPath::parse("/peripherals[type=\"relay\"][default=0]").unwrap();
    assert_eq!(
        q3.segments,
        vec![
            Segment::Key("peripherals".to_string()),
            Segment::Predicates(vec![
                Predicate {
                    key: "type".to_string(),
                    op: PredicateOp::Eq,
                    value: PredicateValue::String("relay".to_string()),
                },
                Predicate {
                    key: "default".to_string(),
                    op: PredicateOp::Eq,
                    value: PredicateValue::Number(0.0),
                }
            ])
        ]
    );
}

#[test]
fn test_parse_wildcards() {
    let q1 = QueryPath::parse("/platform_manager/*/id").unwrap();
    assert_eq!(
        q1.segments,
        vec![
            Segment::Key("platform_manager".to_string()),
            Segment::Wildcard,
            Segment::Key("id".to_string())
        ]
    );

    let q2 = QueryPath::parse("/**/pin").unwrap();
    assert_eq!(
        q2.segments,
        vec![Segment::RecursiveWildcard, Segment::Key("pin".to_string())]
    );
}

#[test]
fn test_parse_all_comparison_operators() {
    assert_eq!(
        QueryPath::parse("/test[val>10]").unwrap().segments,
        vec![
            Segment::Key("test".to_string()),
            Segment::Predicates(vec![Predicate {
                key: "val".to_string(),
                op: PredicateOp::Gt,
                value: PredicateValue::Number(10.0),
            }])
        ]
    );
    assert_eq!(
        QueryPath::parse("/test[val<=5.5]").unwrap().segments,
        vec![
            Segment::Key("test".to_string()),
            Segment::Predicates(vec![Predicate {
                key: "val".to_string(),
                op: PredicateOp::Lte,
                value: PredicateValue::Number(5.5),
            }])
        ]
    );
    assert_eq!(
        QueryPath::parse("/test[val!=0]").unwrap().segments,
        vec![
            Segment::Key("test".to_string()),
            Segment::Predicates(vec![Predicate {
                key: "val".to_string(),
                op: PredicateOp::NotEq,
                value: PredicateValue::Number(0.0),
            }])
        ]
    );
}
