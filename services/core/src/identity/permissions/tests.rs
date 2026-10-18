use super::*;

fn actor(role: PlatformRole) -> Actor {
    Actor {
        user_id: 7,
        personal_account_id: 8,
        role,
        credential: Credential::Session,
        active: true,
        account_active: true,
        auth_version: 3,
        credential_auth_version: 3,
        developer: DeveloperState::Granted,
    }
}
fn policy(overrides: &[Override]) -> Policy<'_> {
    Policy {
        subject_user_id: 7,
        contract: CONTRACT_VERSION,
        revision: 1,
        sha256: [9; 32],
        verified_complete: true,
        overrides,
    }
}
fn member(role: MemberRole, can_spend: bool) -> Membership {
    Membership {
        user_id: 7,
        team_id: 4,
        account_id: 5,
        role,
        can_spend,
        active: true,
        revision: 1,
    }
}

#[test]
fn root_catalog_allows_known_actions_and_unknown_stays_closed() {
    let decision = decide(
        &actor(PlatformRole::Root),
        &policy(&[]),
        Action::ChannelSecretView,
        Scope::Platform(PlatformResource::Channel),
        None,
    );
    assert_eq!(decision, PermissionDecision::Allow);
    let unknown = decide(
        &actor(PlatformRole::Root),
        &policy(&[]),
        Action::Unknown,
        Scope::Platform(PlatformResource::Channel),
        None,
    );
    assert_eq!(unknown, PermissionDecision::Deny(Denial::UnknownAction));
}

#[test]
fn explicit_deny_beats_admin_default_and_api_key_cannot_administer() {
    let deny_row = [Override {
        subject_user_id: 7,
        action: Action::ChannelRead,
        effect: Effect::Deny,
    }];
    let denied = decide(
        &actor(PlatformRole::Admin),
        &policy(&deny_row),
        Action::ChannelRead,
        Scope::Platform(PlatformResource::Channel),
        None,
    );
    assert_eq!(denied, PermissionDecision::Deny(Denial::ExplicitDeny));
    let mut key = actor(PlatformRole::Root);
    key.credential = Credential::ApiKey;
    let keyed = decide(
        &key,
        &policy(&[]),
        Action::ChannelRead,
        Scope::Platform(PlatformResource::Channel),
        None,
    );
    assert_eq!(keyed, PermissionDecision::Deny(Denial::CredentialKind));
}

#[test]
fn complete_catalog_snapshot_hashes_in_fixed_order() {
    let entries: Vec<(String, String)> = Action::CATALOG
        .iter()
        .enumerate()
        .map(|(index, action)| {
            (
                action.catalog_name().unwrap().to_owned(),
                if index < 5 { "allow" } else { "deny" }.to_owned(),
            )
        })
        .collect();
    let snapshot = complete_policy_snapshot(7, 1, &entries).unwrap();
    assert_eq!(
        policy_sha256(&snapshot).unwrap(),
        policy_sha256(&complete_policy_snapshot(7, 1, &entries).unwrap()).unwrap()
    );
    assert!(matches!(
        complete_policy_snapshot(7, 1, &entries[..8]),
        Err(PolicySnapshotError::Incomplete)
    ));
    let mut unknown = entries.clone();
    unknown[0].0 = "NotAnAction".into();
    assert_eq!(
        complete_policy_snapshot(7, 1, &unknown),
        Err(PolicySnapshotError::UnknownAction)
    );
}

#[test]
fn team_spend_and_cross_account_profile_are_scoped() {
    let scope = Scope::Team {
        team_id: 4,
        account_id: 5,
    };
    let plain = member(MemberRole::Member, false);
    let spend = decide(
        &actor(PlatformRole::User),
        &policy(&[]),
        Action::TeamSpend,
        scope,
        Some(&plain),
    );
    assert_eq!(spend, PermissionDecision::Deny(Denial::RoleDenied));
    let owner = member(MemberRole::Owner, false);
    let allowed = decide(
        &actor(PlatformRole::User),
        &policy(&[]),
        Action::TeamSpend,
        scope,
        Some(&owner),
    );
    assert_eq!(allowed, PermissionDecision::Allow);
    let other = Scope::User {
        user_id: 99,
        account_id: 100,
        role: PlatformRole::User,
    };
    let crossed = decide(
        &actor(PlatformRole::User),
        &policy(&[]),
        Action::OwnProfileRead,
        other,
        None,
    );
    assert_eq!(crossed, PermissionDecision::Deny(Denial::TenantDenied));
}

#[test]
fn every_catalog_action_uses_main_role_defaults_and_deny_precedence() {
    for action in Action::CATALOG {
        let scope = Scope::Platform(platform_kind(action).unwrap());
        for role in [PlatformRole::User, PlatformRole::Admin, PlatformRole::Root] {
            let baseline = decide(&actor(role), &policy(&[]), action, scope, None);
            // Fixed main resources_channel.go/resources_acquisition.go grant
            // these five operations to admin; sensitive/detail grants need overrides.
            let main_admin_baseline = [
                Action::ChannelRead,
                Action::ChannelOperate,
                Action::ChannelWrite,
                Action::AcquisitionRead,
                Action::AcquisitionWrite,
            ]
            .contains(&action);
            assert_eq!(
                baseline.allowed(),
                role == PlatformRole::Root || (role == PlatformRole::Admin && main_admin_baseline)
            );
            let conflicting = [
                Override {
                    subject_user_id: 7,
                    action,
                    effect: Effect::Allow,
                },
                Override {
                    subject_user_id: 7,
                    action,
                    effect: Effect::Deny,
                },
            ];
            let result = decide(&actor(role), &policy(&conflicting), action, scope, None);
            assert_eq!(result.allowed(), role == PlatformRole::Root);
        }
    }
}

#[test]
fn catalog_grants_never_change_resource_or_credential_authority() {
    let rows = Action::CATALOG.map(|action| Override {
        subject_user_id: 7,
        action,
        effect: Effect::Allow,
    });
    for role in [PlatformRole::Admin, PlatformRole::Root] {
        for action in Action::CATALOG {
            let right = Scope::Platform(platform_kind(action).unwrap());
            let wrong = Scope::Platform(PlatformResource::PermissionCatalog);
            assert_eq!(
                decide(&actor(role), &policy(&rows), action, wrong, None),
                PermissionDecision::Deny(Denial::InvalidScope)
            );
            assert_eq!(
                decide(&actor(role), &policy(&rows), action, Scope::Empty, None),
                PermissionDecision::Deny(Denial::InvalidScope)
            );
            for credential in [Credential::ApiKey, Credential::Host] {
                let mut non_session = actor(role);
                non_session.credential = credential;
                assert_eq!(
                    decide(&non_session, &policy(&rows), action, right, None),
                    PermissionDecision::Deny(Denial::CredentialKind)
                );
            }
            let mut stale = actor(role);
            stale.credential_auth_version -= 1;
            assert_eq!(
                decide(&stale, &policy(&rows), action, right, None),
                PermissionDecision::Deny(Denial::StaleAuthority)
            );
        }
    }
}

#[test]
fn unrelated_or_unknown_policy_rows_fail_closed_for_root_too() {
    for action in [Action::Unknown, Action::OwnProfileRead] {
        let rows = [Override {
            subject_user_id: 7,
            action,
            effect: Effect::Allow,
        }];
        assert_eq!(
            decide(
                &actor(PlatformRole::Root),
                &policy(&rows),
                Action::ChannelRead,
                Scope::Platform(PlatformResource::Channel),
                None
            ),
            PermissionDecision::Deny(Denial::PolicyUnavailable)
        );
    }
    let rows = [Override {
        subject_user_id: 8,
        action: Action::ChannelRead,
        effect: Effect::Allow,
    }];
    assert_eq!(
        decide(
            &actor(PlatformRole::Root),
            &policy(&rows),
            Action::ChannelRead,
            Scope::Platform(PlatformResource::Channel),
            None
        ),
        PermissionDecision::Deny(Denial::PolicyUnavailable)
    );
}
