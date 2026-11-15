use super::*;
use serde::{Deserialize, Serialize};

const SAMPLE: &str = r#"
name = "kkt"
count = 3
ratio = 0.5
on = true
when = 1979-05-27T07:32:00Z
day = 1979-05-27
hosts = ["a", "b"]
point = { x = 1, y = "two" }

[server]
url = "http://a/mcp"
"#;

#[test]
fn parse_gives_each_kind_of_value() {
    let table = parse(SAMPLE).expect("parses");
    assert_eq!(table["name"], Value::Text("kkt".into()));
    assert_eq!(table["count"], Value::Integer(3));
    assert_eq!(table["ratio"], Value::Float(0.5));
    assert_eq!(table["on"], Value::Boolean(true));
    assert_eq!(
        table["when"],
        Value::DateTime("1979-05-27T07:32:00Z".into())
    );
    assert_eq!(table["day"], Value::DateTime("1979-05-27".into()));
    assert_eq!(
        table["hosts"],
        Value::List(vec![Value::Text("a".into()), Value::Text("b".into())])
    );
    let server = table["server"].as_table().expect("a table");
    assert_eq!(server["url"].as_text(), Some("http://a/mcp"));
}

#[test]
fn keys_come_in_name_order() {
    let table = parse("b = 1\na = 2\nc = 3\n").expect("parses");
    assert_eq!(table.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
}

/// Each value prints exactly as the library prints the same value, which
/// is what kkt's messages have always shown.
#[test]
fn values_print_as_the_library_prints_them() {
    let ours = parse(SAMPLE).expect("parses");
    let library: toml::Table = SAMPLE.parse().expect("parses");
    for (key, value) in &library {
        assert_eq!(ours[key].to_string(), value.to_string(), "{key}");
    }
    let shown: Vec<_> = [
        "name", "count", "ratio", "on", "when", "day", "hosts", "point",
    ]
    .iter()
    .map(|key| ours[*key].to_string())
    .collect();
    assert_eq!(
        shown,
        [
            r#""kkt""#,
            "3",
            "0.5",
            "true",
            "1979-05-27T07:32:00Z",
            "1979-05-27",
            r#"["a", "b"]"#,
            r#"{ x = 1, y = "two" }"#,
        ]
    );
}

/// The end-to-end tests turn a parsed tree into JSON; it comes out as the
/// library's tree did, except that a date-time is its text.
#[test]
fn a_tree_turns_into_the_same_json_as_the_librarys() {
    let text = "a = 1\nb = [\"x\", 2.5, true]\n[c]\nd = { e = \"f\" }\n";
    let ours = serde_json::to_value(parse(text).expect("parses")).expect("json");
    let library = serde_json::to_value(text.parse::<toml::Table>().expect("parses")).expect("json");
    assert_eq!(ours, library);
    let when = serde_json::to_value(parse("w = 1979-05-27\n").expect("parses")).expect("json");
    assert_eq!(when, serde_json::json!({"w": "1979-05-27"}));
}

#[test]
fn a_parse_error_gives_the_parsers_reason_and_place() {
    let text = "a = 1\nb = \n";
    let ours = parse(text).expect_err("malformed");
    let library = text.parse::<toml::Table>().expect_err("malformed");
    assert_eq!(ours.reason(), library.message());
    assert_eq!(ours.to_string(), library.to_string());
    assert_eq!(ours.place().map(|p| p.line), Some(2));
}

#[test]
fn a_place_counts_columns_in_characters() {
    // The second `=` on line 2 is the 9th character but the 10th byte.
    let error = Document::parse("a = 1\n'\u{e9}' = 2 = 3\n").expect_err("malformed");
    assert_eq!(error.place(), Some(Place { line: 2, column: 9 }));
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    name: String,
    count: u32,
}

#[test]
fn a_record_is_written_and_read_back() {
    let record = Record {
        name: "kkt".into(),
        count: 2,
    };
    let text = write(&record).expect("writes");
    assert_eq!(text, "name = \"kkt\"\ncount = 2\n");
    assert_eq!(read::<Record>(&text).expect("reads"), record);
}

#[test]
fn reading_a_record_gives_the_libraries_reason() {
    let text = "name = 3\n";
    let ours = read::<Record>(text).expect_err("wrong type");
    let library = toml::from_str::<Record>(text).expect_err("wrong type");
    assert_eq!(ours.reason(), library.message());
    assert_eq!(ours.to_string(), library.to_string());
}

#[test]
fn writing_a_record_that_toml_cannot_hold_gives_the_writers_reason() {
    let ours = write(&3).expect_err("not a table");
    let library = toml::to_string(&3).expect_err("not a table");
    assert_eq!(ours.to_string(), library.to_string());
}

mod editing {
    use super::*;

    const CODEX: &str = "# my settings\nmodel = \"o3\" # keep\n\n[mcp_servers.kkt]\nurl = \"http://old/mcp\" # the server\ncommand = \"x\"\n\n[mcp_servers.kkt.http_headers]\nAuthorization = \"secret\"\nX-Other = \"y\"\n";

    #[test]
    fn untouched_text_comes_back_as_it_was() {
        let document = Document::parse(CODEX).expect("parses");
        assert_eq!(document.text(), CODEX);
    }

    #[test]
    fn find_looks_through_tables() {
        let document = Document::parse(CODEX).expect("parses");
        assert_eq!(document.find(&["mcp_servers", "kkt"]), Some(Found::Table));
        assert_eq!(
            document.find(&["mcp_servers", "kkt", "url"]),
            Some(Found::Text("http://old/mcp".into()))
        );
        assert_eq!(document.find(&["model", "x"]), None);
        assert_eq!(document.find(&["mcp_servers", "other"]), None);
        assert_eq!(document.find(&[]), None);
        let inline = Document::parse("a = { b = 1 }\n").expect("parses");
        assert_eq!(inline.find(&["a"]), Some(Found::Table));
        assert_eq!(inline.find(&["a", "b"]), Some(Found::Other));
    }

    #[test]
    fn changes_keep_the_rest_of_the_text() {
        let mut document = Document::parse(CODEX).expect("parses");
        let mut kkt = document
            .table_mut(&["mcp_servers", "kkt"], IfMissing::Refuse)
            .expect("there");
        kkt.remove("command");
        kkt.set_text("url", "http://new/mcp");
        let mut headers = kkt
            .table_mut(&["http_headers"], IfMissing::Refuse)
            .expect("there");
        assert_eq!(headers.keys(), ["Authorization", "X-Other"]);
        headers.remove("Authorization");
        assert_eq!(
            document.text(),
            "# my settings\nmodel = \"o3\" # keep\n\n[mcp_servers.kkt]\nurl = \"http://new/mcp\"\n\n[mcp_servers.kkt.http_headers]\nX-Other = \"y\"\n"
        );
    }

    #[test]
    fn missing_tables_are_added_or_refused_as_asked() {
        let mut document = Document::parse("a = 1\n").expect("parses");
        assert_eq!(
            document.table_mut(&["s", "t"], IfMissing::Refuse).err(),
            Some(EditError::Missing {
                path: vec!["s".into()]
            })
        );
        document
            .table_mut(&["s", "t"], IfMissing::Add)
            .expect("added")
            .set_text("k", "v");
        assert_eq!(document.text(), "a = 1\n\n[s]\n\n[s.t]\nk = \"v\"\n");
    }

    #[test]
    fn a_key_that_is_not_a_table_is_named_at_its_level() {
        let mut document = Document::parse("s = 1\n[t]\nu = 2\n").expect("parses");
        for if_missing in [IfMissing::Add, IfMissing::Refuse] {
            assert_eq!(
                document.table_mut(&["s", "x"], if_missing).err(),
                Some(EditError::NotATable {
                    path: vec!["s".into()]
                })
            );
            assert_eq!(
                document.table_mut(&["t", "u"], if_missing).err(),
                Some(EditError::NotATable {
                    path: vec!["t".into(), "u".into()]
                })
            );
        }
        assert_eq!(
            document.table_mut(&["t", "v"], IfMissing::Refuse).err(),
            Some(EditError::Missing {
                path: vec!["t".into(), "v".into()]
            })
        );
    }

    #[test]
    fn an_inline_table_is_a_table() {
        let mut document = Document::parse("[s]\nt = { a = \"b\" }\n").expect("parses");
        document
            .table_mut(&["s", "t"], IfMissing::Refuse)
            .expect("inline")
            .set_text("c", "d");
        assert_eq!(document.text(), "[s]\nt = { a = \"b\" , c = \"d\" }\n");
    }

    /// The written form is the value's own text, as the library gives it.
    #[test]
    fn written_is_the_values_text_as_the_library_gives_it() {
        let text = "[s]\nmode = \"x\" # note\nlist = [1, 2]\nn = 3\n[s.deep]\n";
        let mut document = Document::parse(text).expect("parses");
        let library: toml_edit::DocumentMut = text.parse().expect("parses");
        let table = document
            .table_mut(&["s"], IfMissing::Refuse)
            .expect("there");
        for key in ["mode", "list", "n"] {
            assert_eq!(
                table.written(key),
                library["s"][key].as_value().map(|v| v.to_string()),
                "{key}"
            );
        }
        // The comment after a value is part of its text, as kkt has always shown it.
        assert_eq!(
            table.written("mode").as_deref().map(str::trim),
            Some("\"x\" # note")
        );
        assert_eq!(table.written("deep"), None);
        assert_eq!(table.written("missing"), None);
    }

    #[test]
    fn a_list_is_added_to_once() {
        let mut document = Document::parse("# hosts\n").expect("parses");
        let mut hosts = document.list_mut("allow_hosts").expect("added");
        assert!(!hosts.contains_text("a"));
        hosts.push_text("a");
        assert!(hosts.contains_text("a"));
        // A key added at the top goes before the comment, as the library places it.
        assert_eq!(document.text(), "allow_hosts = [\"a\"]\n# hosts\n");
        let mut document = Document::parse("allow_hosts = 1\n").expect("parses");
        assert_eq!(
            document.list_mut("allow_hosts").err(),
            Some(EditError::NotAList {
                path: vec!["allow_hosts".into()]
            })
        );
    }
}
