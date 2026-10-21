use super::*;

const DB: &str = "[database]\nurl_file='/nonexistent-lmm-test/db'\n";
const WEB: &str = "[identity.web]\norigin='https://telegram.example.invalid'\nrefresh_key_file='/nonexistent-lmm-test/refresh'\nuser_agreement_enabled=true\nprivacy_policy_enabled=true\n";
const TELEGRAM: &str = "[identity.telegram]\nbot_name='lmm_test_bot'\nbot_token_file='/nonexistent-lmm-test/telegram'\n";

#[test]
fn deployment_telegram_requires_database_web_and_private_file_reference() {
    for prefix in [String::new(), DB.to_owned(), WEB.to_owned()] {
        assert!(parse_core_file(format!("version=1\n{prefix}{TELEGRAM}").as_bytes()).is_err());
    }
    let document = format!("version=1\n{DB}{WEB}{TELEGRAM}");
    let values = parse_core_file(document.as_bytes()).unwrap();
    assert_eq!(
        values.get("LMM_CORE_TELEGRAM_BOT_NAME"),
        Some(&OsString::from("lmm_test_bot"))
    );
    assert_eq!(
        values.get("LMM_CORE_TELEGRAM_BOT_TOKEN_FILE"),
        Some(&OsString::from("/nonexistent-lmm-test/telegram"))
    );
    assert!(!values.contains_key("LMM_CORE_TELEGRAM_BOT_TOKEN"));
    assert!(
        parse_core_file(
            document
                .replace("'/nonexistent-lmm-test/telegram'", "'relative-token'")
                .as_bytes()
        )
        .is_err()
    );
}

#[test]
fn deployment_telegram_does_not_accept_inline_or_oauth_credentials() {
    let base = format!("version=1\n{DB}{WEB}");
    for fields in [
        "bot_name='lmm_test_bot'\nbot_token='synthetic-inline-disallowed'\n",
        "bot_name='lmm_test_bot'\nclient_secret_file='/nonexistent-lmm-test/telegram'\n",
        "bot_name='lmm_test_bot'\n",
    ] {
        assert!(
            parse_core_file(format!("{base}[identity.telegram]\n{fields}").as_bytes()).is_err()
        );
    }
}
