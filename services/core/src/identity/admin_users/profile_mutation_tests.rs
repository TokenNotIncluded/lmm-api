use super::*;

fn request(version: &str, expected: &str, name: &str) -> ProfileUpdateRequest {
    ProfileUpdateRequest {
        expected_auth_version: version.to_owned(),
        expected_display_name: expected.to_owned(),
        display_name: name.to_owned(),
    }
}

#[test]
fn new_name_uses_code_points_bytes_and_controls_without_normalizing() {
    for name in ["".to_owned(), " 名字 ".to_owned(), "💡".repeat(20)] {
        let input = request("7", "before", &name);
        assert_eq!(input.validate().unwrap().new_name, name);
    }
    for name in [
        "a".repeat(21),
        "💡".repeat(21),
        "a\n".to_owned(),
        "\u{007f}".to_owned(),
        "\u{0085}".to_owned(),
    ] {
        assert_eq!(
            request("7", "before", &name).validate().err(),
            Some(ProfileError::Invalid)
        );
    }
    // Ten decomposed accented characters are twenty scalar values. No NFC
    // normalization silently changes the submitted CAS or display value.
    let decomposed = "e\u{0301}".repeat(10);
    assert_eq!(
        request("7", "before", &decomposed)
            .validate()
            .unwrap()
            .new_name,
        decomposed
    );
}

#[test]
fn historical_names_can_be_repaired_without_accepting_them_as_new_values() {
    let old = "💡".repeat(256);
    assert!(request("7", &old, "").validate().is_ok());
    assert!(request("7", "historical\nname", "fixed").validate().is_ok());
    assert_eq!(
        request("7", &"💡".repeat(257), "fixed").validate().err(),
        Some(ProfileError::Invalid)
    );
    assert_eq!(
        request("7", "old", &old).validate().err(),
        Some(ProfileError::Invalid)
    );
}

#[test]
fn auth_version_requires_exact_positive_i64_decimal_string() {
    for version in ["1", "9223372036854775807"] {
        assert!(request(version, "old", "new").validate().is_ok());
    }
    for version in [
        "",
        "0",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "1.0",
        "9223372036854775808",
        "11111111111111111111",
    ] {
        assert_eq!(
            request(version, "old", "new").validate().err(),
            Some(ProfileError::Invalid)
        );
    }
}

#[test]
fn strict_json_rejects_extra_duplicate_missing_and_numeric_fields() {
    let valid =
        r#"{"expected_auth_version":"7","expected_display_name":"old","display_name":"new"}"#;
    assert!(serde_json::from_str::<ProfileUpdateRequest>(valid).is_ok());
    for input in [
        r#"{"expected_auth_version":"7","expected_display_name":"old","display_name":"new","role":"superadmin"}"#,
        r#"{"expected_auth_version":"7","expected_auth_version":"8","expected_display_name":"old","display_name":"new"}"#,
        r#"{"expected_auth_version":"7","expected_display_name":"old","display_name":"new","display_name":"other"}"#,
        r#"{"expected_auth_version":"7","expected_display_name":"old","expected_display_name":"other","display_name":"new"}"#,
        r#"{"expected_auth_version":7,"expected_display_name":"old","display_name":"new"}"#,
        r#"{"expected_auth_version":"7","display_name":"new"}"#,
        r#"{"expected_auth_version":"7","expected_display_name":"old","display_name":null}"#,
    ] {
        assert!(serde_json::from_str::<ProfileUpdateRequest>(input).is_err());
    }
}

#[test]
fn both_cas_preconditions_are_checked_before_no_op() {
    let change = request("7", "old", "new");
    assert_eq!(
        should_change(7, "old", &change.validate().unwrap()),
        Ok(true)
    );
    assert_eq!(
        should_change(8, "old", &change.validate().unwrap()),
        Err(ProfileError::Conflict)
    );
    assert_eq!(
        should_change(7, "other", &change.validate().unwrap()),
        Err(ProfileError::Conflict)
    );
    assert_eq!(
        should_change(7, "new", &change.validate().unwrap()),
        Err(ProfileError::Conflict)
    );
    let no_op = request("7", "old", "old");
    assert_eq!(
        should_change(7, "old", &no_op.validate().unwrap()),
        Ok(false)
    );
}

#[test]
fn audit_metadata_has_digests_and_ids_without_raw_display_values() {
    let value = audit_target(123, 7, "private-before", "private-after");
    assert_eq!(value["user_id"], 123);
    assert_eq!(value["target_auth_version"], "7");
    assert_eq!(value["before_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(value["after_sha256"].as_str().unwrap().len(), 64);
    assert!(!value.to_string().contains("private-before"));
    assert!(!value.to_string().contains("private-after"));
}

#[test]
fn independent_capability_probe_is_scoped_to_profile_operation() {
    let value = serde_json::to_value(profile_capabilities()).unwrap();
    assert_eq!(value["contract"], PROFILE_CONTRACT);
    assert_eq!(value["target_platform_roles"], serde_json::json!(["user"]));
    assert_eq!(
        value["operations"],
        serde_json::json!({"read":true,"update_display_name":true})
    );
    assert_eq!(
        value["preconditions"],
        serde_json::json!(["auth_version", "display_name"])
    );
    assert!(value.get("implemented_write_actions").is_none());
    assert_eq!(
        value["display_name_limits"],
        serde_json::json!({"max_code_points":20,"max_utf8_bytes":80,"controls_allowed":false})
    );
}

#[test]
fn authentication_header_duplicates_and_error_statuses_fail_closed() {
    let mut headers = HeaderMap::new();
    assert_eq!(bearer(&headers), Err(ProfileError::Unauthorized));
    headers.insert(header::AUTHORIZATION, "Basic value".parse().unwrap());
    assert_eq!(bearer(&headers), Err(ProfileError::Unauthorized));
    headers.insert(header::AUTHORIZATION, "Bearer session".parse().unwrap());
    assert_eq!(bearer(&headers), Ok("session"));
    headers.append(header::AUTHORIZATION, "Bearer another".parse().unwrap());
    assert_eq!(bearer(&headers), Err(ProfileError::Unauthorized));
    for (error, expected) in [
        (ProfileError::Invalid, 400),
        (ProfileError::Unauthorized, 401),
        (ProfileError::Forbidden, 403),
        (ProfileError::NotFound, 404),
        (ProfileError::Conflict, 409),
        (ProfileError::Unavailable, 503),
    ] {
        assert_eq!(error.into_response().status().as_u16(), expected);
    }
}
