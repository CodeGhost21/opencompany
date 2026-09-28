use super::*;
use crate::ports::types::CompanyEvent;
use crate::server::chat_history::owns;

fn decode(line: &str) -> CompanyEvent {
    serde_json::from_str(line).expect("a journaled line loads")
}

#[test]
fn every_legacy_spelling_is_general() {
    for spelling in ["", "general", "General", "GENERAL", "main", "Main"] {
        assert!(is_general_spelling(spelling), "{spelling:?}");
        assert_eq!(decode_general_chat_id(spelling.into()), GENERAL_CHANNEL_ID);
    }
    for other in ["engineering", "dm:ceo", "general_store", "mainframe"] {
        assert!(!is_general_spelling(other), "{other:?}");
        assert_eq!(decode_general_chat_id(other.into()), other);
    }
    assert_eq!(decode_general_chat_opt(None), None);
}

#[test]
fn a_mixed_legacy_journal_reads_as_one_general_transcript() {
    let lines = [
        r#"{"kind":"OperatorMessage","text":"unaddressed"}"#,
        r#"{"kind":"OperatorMessage","text":"from the console","chat":"main"}"#,
        r#"{"kind":"OperatorMessage","text":"now","chat":"general"}"#,
        r#"{"kind":"AgentReply","chat_id":"General","agent_id":"ceo","text":"a"}"#,
        r#"{"kind":"AgentReply","chat_id":"","agent_id":"ceo","text":"b"}"#,
        r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"c"}"#,
    ];
    for line in lines {
        let event = decode(line);
        assert!(
            owns(GENERAL_CHANNEL_ID, GENERAL_CHANNEL_NAME, &event),
            "#general owns {line}"
        );
        assert!(!owns("engineering", "Engineering", &event), "{line}");
    }
    let desk =
        decode(r#"{"kind":"AgentReply","chat_id":"engineering","agent_id":"ceo","text":"d"}"#);
    assert!(!owns(GENERAL_CHANNEL_ID, GENERAL_CHANNEL_NAME, &desk));
}

#[test]
fn a_legacy_chat_id_decodes_to_general_on_every_stamped_field() {
    match decode(r#"{"kind":"AgentReply","chat_id":"main","agent_id":"ceo","text":"hi"}"#) {
        CompanyEvent::AgentReply { chat_id, .. } => assert_eq!(chat_id, GENERAL_CHANNEL_ID),
        other => panic!("{other:?}"),
    }
    match decode(r#"{"kind":"OperatorMessage","text":"hi","chat":"General"}"#) {
        CompanyEvent::OperatorMessage { chat, .. } => {
            assert_eq!(chat.as_deref(), Some(GENERAL_CHANNEL_ID))
        }
        other => panic!("{other:?}"),
    }
    match decode(r#"{"kind":"OperatorMessage","text":"hi"}"#) {
        CompanyEvent::OperatorMessage { chat, .. } => assert_eq!(chat, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_absent_optional_chat_stays_absent() {
    assert_eq!(
        deserialize_general_chat_opt(serde_json::Value::Null).unwrap(),
        None
    );
    assert_eq!(
        deserialize_general_chat_opt(serde_json::json!("main")).unwrap(),
        Some(GENERAL_CHANNEL_ID.to_string())
    );
    assert_eq!(
        deserialize_general_chat(serde_json::json!("")).unwrap(),
        GENERAL_CHANNEL_ID
    );
}
