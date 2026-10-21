use super::*;
use axum::{extract::Query, http::Uri};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write;

const TOKEN: &str = "123456789:synthetic-only-widget-signing-key";
const NOW: i64 = 1_700_000_000;

fn fields(id: &str, auth_date: i64) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("id".into(), id.into()),
        ("auth_date".into(), auth_date.to_string()),
        ("first_name".into(), "Synthetic".into()),
    ])
}

// An independent existing implementation signs the synthetic fixtures; the
// production verifier uses RustCrypto, not this ring signing implementation.
fn signed(mut fields: BTreeMap<String, String>) -> TelegramAuthorization {
    let data = fields
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let key = ring::digest::digest(&ring::digest::SHA256, TOKEN.as_bytes());
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_ref());
    let tag = ring::hmac::sign(&key, data.as_bytes());
    let mut hash = String::with_capacity(64);
    for byte in tag.as_ref() {
        write!(&mut hash, "{byte:02x}").unwrap();
    }
    fields.insert("hash".into(), hash);
    serde_json::from_value(serde_json::to_value(fields).unwrap()).unwrap()
}

fn verifier() -> TelegramLogin {
    TelegramLogin::new("lmm_test_bot", TOKEN).unwrap()
}

#[test]
fn ordinary_widget_signature_accepts_modern_ids_and_preserves_exact_optional_fields() {
    let login = verifier();
    let mut values = fields("4503599627370495", NOW);
    values.insert("first_name".into(), "  Synthetic &=+% 中文  ".into());
    values.insert("last_name".into(), "Example".into());
    values.insert("username".into(), "synthetic_user".into());
    values.insert(
        "photo_url".into(),
        "https://example.invalid/a?b=c&d=e".into(),
    );
    values.insert("lang".into(), "zh-Hans".into());
    let mut input = signed(values);
    let identity = login.verify(&input, NOW).unwrap();
    assert_eq!(identity.subject(), "4503599627370495");
    assert_eq!(identity.auth_date(), NOW);
    let original_digest = identity.replay_digest();
    assert!(original_digest.iter().any(|byte| *byte != 0));
    input.hash.make_ascii_uppercase();
    assert!(login.verify(&input, NOW).unwrap().replay_digest() == original_digest);
    input.first_name = Some(input.first_name.as_deref().unwrap().trim().into());
    assert!(login.verify(&input, NOW).is_err());
    assert_eq!(
        TelegramLogin::new("@lmm_test_bot", TOKEN)
            .unwrap()
            .bot_name(),
        "lmm_test_bot"
    );
    assert_eq!(
        login
            .verify(&signed(fields(&i64::MAX.to_string(), NOW)), NOW)
            .unwrap()
            .subject(),
        i64::MAX.to_string()
    );
}

#[test]
fn widget_age_boundary_is_five_minutes_and_future_skew_is_at_most_thirty_seconds() {
    let login = verifier();
    for (offset, allowed) in [
        (-301, false),
        (-300, false),
        (-299, true),
        (0, true),
        (30, true),
        (31, false),
    ] {
        let input = signed(fields("123456789", NOW + offset));
        assert_eq!(login.verify(&input, NOW).is_ok(), allowed);
    }
    let input = signed(fields("123456789", NOW));
    assert!(login.verify(&input, 0).is_err());
    assert!(login.verify(&input, -1).is_err());
    assert!(login.verify(&input, i64::MAX).is_err());
}

#[test]
fn signed_noncanonical_or_out_of_range_numeric_fields_are_rejected() {
    let login = verifier();
    for invalid in [
        "",
        "0",
        "-1",
        "+1",
        "01",
        " 1",
        "1 ",
        "1.0",
        "١",
        "9223372036854775808",
    ] {
        assert!(login.verify(&signed(fields(invalid, NOW)), NOW).is_err());
    }
    for invalid in [
        "0",
        "-1",
        "+1700000000",
        "01700000000",
        " 1700000000",
        "1700000000 ",
        "1700000000.0",
        "9223372036854775808",
    ] {
        let mut values = fields("123456789", NOW);
        values.insert("auth_date".into(), invalid.into());
        assert!(login.verify(&signed(values), NOW).is_err());
    }
}

#[test]
fn every_supplied_widget_field_and_the_exact_bot_key_are_authenticated() {
    let login = verifier();
    for field in [
        "id",
        "auth_date",
        "first_name",
        "last_name",
        "username",
        "photo_url",
        "lang",
    ] {
        let mut input = signed(fields("123456789", NOW));
        match field {
            "id" => input.id = "123456788".into(),
            "auth_date" => input.auth_date = (NOW - 1).to_string(),
            "first_name" => input.first_name = Some("Changed".into()),
            "last_name" => input.last_name = Some("Changed".into()),
            "username" => input.username = Some("changed".into()),
            "photo_url" => input.photo_url = Some("https://example.invalid/changed".into()),
            "lang" => input.lang = Some("en".into()),
            _ => unreachable!(),
        }
        assert!(login.verify(&input, NOW).is_err());
    }
    let input = signed(fields("123456789", NOW));
    let wrong = TelegramLogin::new("lmm_test_bot", "different-synthetic-key").unwrap();
    assert!(wrong.verify(&input, NOW).is_err());
    for hash in [
        "",
        "00",
        &"0".repeat(64),
        &"g".repeat(64),
        &"é".repeat(32),
        &"a".repeat(65),
    ] {
        let mut input = signed(fields("123456789", NOW));
        input.hash = hash.into();
        assert!(login.verify(&input, NOW).is_err());
    }
}

#[test]
fn signed_optional_field_size_limits_and_control_characters_are_rejected() {
    let login = verifier();
    for (name, maximum) in [
        ("first_name", 256),
        ("last_name", 256),
        ("username", 64),
        ("photo_url", 2048),
        ("lang", 64),
    ] {
        let mut values = fields("123456789", NOW);
        values.insert(name.into(), "x".repeat(maximum));
        assert!(login.verify(&signed(values.clone()), NOW).is_ok());
        values.insert(name.into(), "x".repeat(maximum + 1));
        assert!(login.verify(&signed(values), NOW).is_err());
        for invalid in ["line\nbreak", "line\rbreak", "null\0byte", "delete\u{007f}"] {
            let mut values = fields("123456789", NOW);
            values.insert(name.into(), invalid.into());
            assert!(login.verify(&signed(values), NOW).is_err());
        }
    }
    let mut values = fields("123456789", NOW);
    values.insert("last_name".into(), String::new());
    let mut input = signed(values);
    assert!(login.verify(&input, NOW).is_ok());
    input.last_name = None;
    assert!(login.verify(&input, NOW).is_err());
}

#[test]
fn schema_rejects_unknown_missing_duplicate_and_non_widget_fields() {
    let input = signed(fields("123456789", NOW));
    let base = json!({"id":input.id,"auth_date":input.auth_date,"hash":input.hash});
    for field in [
        "state",
        "nonce",
        "user",
        "id_token",
        "query_id",
        "start_param",
    ] {
        let mut value = base.clone();
        value[field] = json!("must-not-be-trusted");
        assert!(serde_json::from_value::<TelegramAuthorization>(value).is_err());
    }
    for missing in ["id", "auth_date", "hash"] {
        let mut value = base.clone();
        value.as_object_mut().unwrap().remove(missing);
        assert!(serde_json::from_value::<TelegramAuthorization>(value).is_err());
    }
    for name in ["id", "auth_date", "hash", "first_name"] {
        let mut uri = reqwest::Url::parse("https://example.invalid/callback").unwrap();
        for (name, value) in base.as_object().unwrap() {
            uri.query_pairs_mut()
                .append_pair(name, value.as_str().unwrap());
        }
        uri.query_pairs_mut().append_pair(name, "duplicate");
        if name == "first_name" {
            uri.query_pairs_mut().append_pair(name, "second");
        }
        let uri: Uri = uri.as_str().parse().unwrap();
        assert!(Query::<TelegramAuthorization>::try_from_uri(&uri).is_err());
    }
    for field in ["id", "auth_date", "hash"] {
        let mut value = base.clone();
        value[field] = Value::Null;
        assert!(serde_json::from_value::<TelegramAuthorization>(value).is_err());
    }
}

#[test]
fn configuration_is_bounded_and_does_not_accept_whitespace_or_non_ascii_keys() {
    for name in [
        "",
        "@",
        "tiny",
        "name with spaces",
        "非法_bot",
        &"x".repeat(33),
    ] {
        assert!(TelegramLogin::new(name, TOKEN).is_err());
    }
    for token in ["", " ", "key\n", "key\tvalue", "非ascii", &"x".repeat(4097)] {
        assert!(TelegramLogin::new("lmm_test_bot", token).is_err());
    }
    assert!(TelegramLogin::new("lmm_test_bot", &"x".repeat(4096)).is_ok());
}
