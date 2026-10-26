#!/usr/bin/env python3
"""Plan legacy routing offline; never connect, apply, install secrets or approve.

Reuse migrate-legacy-financial.py's hash-bound frozen export and identity map.
Optional JSON contracts have format_version=1, the scope below, and bindings
{source_archive_sha256, source_export_sha256, identity_map_file_sha256}:

* legacy-routing-runtime-evidence: source_revision (full git SHA),
  evidence_reference, memory_cache_enabled, model_matching (exact or
  legacy-normalized-fallback), max_attempts, streaming_enabled, and limits
  {input_token_ceiling, lease_seconds, timeout_seconds, max_output_tokens}.
* legacy-routing-provider-contracts: channels[] containing channel_id,
  source_channel_sha256, protocol, endpoint, legacy_path_prefix (explicit /v1
  or /v1beta from the pinned adapter), contract_revision, models[]
  {source_model, upstream_model, source_operations (e.g. ["chat.completions"]),
  max_input_tokens, max_output_tokens,
  missing_cached_tokens_is_zero, missing_cache_creation_tokens_is_zero;
  Gemini additionally needs legacy_api_version="v1beta"}, and
  prices[] {group, model, kind=flat_tokens, source_options_sha256,
  contract_revision, rates={input,output,cache_read,cache_write,request},
  group_multiplier_ppm, speed_multiplier_ppm}.
* legacy-routing-credential-references: credentials[] containing channel_id,
  source_channel_sha256, source_key_sha256 (SHA256 of the exact UTF-8 field),
  reference={id,revision}, secret_file (absolute installation destination).

Missing/unsupported semantics produce a blocked manifest, never a partial
serving configuration. A candidate is still unvalidated: these supplied
contracts are provenance, not signatures, approvals or Core acceptance.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import unicodedata
from urllib.parse import urlsplit

_SPEC = importlib.util.spec_from_file_location("routing_financial_source", Path(__file__).with_name("migrate-legacy-financial.py"))
F = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(F)
MappingError, require = F.MappingError, F.require
MAX_ID = 2**63 - 1
PROTOCOLS = {1: {"openai_chat", "openai_responses"}, 14: {"anthropic_messages"}, 24: {"gemini"}}
EMPTY = (None, "", "{}", "null")
CHANNEL_FIELDS = {"id", "type", "key", "open_ai_organization", "test_model", "status", "name", "weight",
                  "created_time", "test_time", "response_time", "base_url", "other", "balance", "balance_updated_time",
                  "models", "group", "used_quota", "model_mapping", "status_code_mapping", "priority", "auto_ban",
                  "other_info", "tag", "setting", "param_override", "header_override", "remark", "channel_info",
                  "settings", "public_relay_contribution_id"}
SPECIAL_FIELDS = ("open_ai_organization", "other", "model_mapping", "status_code_mapping", "setting",
                  "param_override", "header_override", "settings")


def number(value, code, minimum=1, maximum=MAX_ID):
    return F.integer(value, code, minimum=minimum, maximum=maximum)


def label(value):
    return (type(value) is str and 0 < len(value) <= 64 and len(value.encode()) <= 256
            and all((c.isascii() and (c.isalnum() or c in "._-/()[]"))
                    or (not c.isascii() and unicodedata.category(c)[0] in "LMNS") for c in value)
            and value != "auto")


def model_name(value):
    return (type(value) is str and re.fullmatch(r"[A-Za-z0-9._/:@~-]{1,200}", value) is not None
            and not value.endswith(("/fast", "/ultrafast")))


def unique(rows, field, code):
    require(type(rows) is list, code)
    result = {}
    for row in rows:
        require(type(row) is dict, code)
        key = number(row.get(field), code)
        require(key not in result, code)
        result[key] = row
    return result


def contract(document, scope, bindings):
    if document is None:
        return None
    require(type(document) is dict and type(document.get("format_version")) is int
            and document["format_version"] == 1 and document.get("scope") == scope, "routing_contract_shape_invalid")
    require(F.digest(document.get("bindings")) == F.digest(bindings), "routing_contract_binding_mismatch")
    return document


def effective_weights(rows, memory_cache):
    """Return actual random-ticket masses, not the legacy intended ratio."""
    weights = {row["channel_id"]: number(row.get("weight"), "routing_weight_invalid", 0, 2**32-1) for row in rows}
    if memory_cache:
        return {cid: weight if any(weights.values()) else 100 for cid, weight in weights.items()}, None
    if len(weights) > 1 and len(set(weights.values())) != len(weights):
        return {}, "legacy_database_tie_order_not_defined"
    ordered = sorted(weights, key=weights.get, reverse=True)
    result = {cid: weight + 10 for cid, weight in weights.items()}
    if len(ordered) > 1:
        # rand.Intn(sum) and <=0 give the first/last channels +1/-1.
        result[ordered[0]] += 1
        result[ordered[-1]] -= 1
    if any(weight > 2**32-1 for weight in result.values()):
        return {}, "native_weight_out_of_range"
    return result, None


def endpoint_valid(value):
    if type(value) is not str or len(value) > 2048 or any(c.isspace() or ord(c) < 32 for c in value):
        return False
    try:
        url = urlsplit(value)
        return (url.scheme == "https" and bool(url.hostname) and not url.username and not url.password
                and not url.query and not url.fragment and not any(c in value for c in "\\@?#")
                and (url.port is None or 1 <= url.port <= 65535))
    except ValueError:
        return False


def blocked_channel(row, provider, credential):
    reasons = []
    number(row.get("type"), "source_channel_type_invalid", 0)
    number(row.get("status"), "source_channel_status_invalid", 0)
    if set(row) - CHANNEL_FIELDS:
        reasons.append("unclassified_channel_columns")
    if row.get("type") not in PROTOCOLS:
        reasons.append("channel_type_not_supported")
    if any(row.get(field) not in EMPTY for field in SPECIAL_FIELDS):
        reasons.append("special_channel_behavior_not_mapped")
    contribution = row.get("public_relay_contribution_id")
    if contribution is not None and not (type(contribution) is int and contribution == 0):
        reasons.append("public_relay_contribution_not_mapped")
    info = row.get("channel_info")
    if type(info) is str and info not in EMPTY:
        info = F.parse_json(info)
    if info not in (None, "", {}) and (type(info) is not dict or info.get("is_multi_key") is not False
            or any(value not in (None, False, 0, "", {}, []) for key, value in info.items() if key != "is_multi_key")):
        reasons.append("multi_key_or_channel_state_not_mapped")
    key = row.get("key")
    if (type(key) is not str or not key or len(key) > 4096 or any(not 33 <= ord(c) <= 126 for c in key)
            or key.startswith(("{", "["))):
        reasons.append("special_credential_not_supported")
    if provider is None:
        reasons.append("provider_contract_missing")
    else:
        require(provider.get("source_channel_sha256") == F.digest(row), "provider_source_channel_changed")
        if provider.get("protocol") not in PROTOCOLS.get(row.get("type"), set()):
            reasons.append("provider_protocol_not_compatible")
        # Legacy adapters append their version prefix; Core appends only the
        # operation. An identical base would silently drop /v1 or /v1beta.
        prefix = provider.get("legacy_path_prefix")
        expected_prefix = "/v1beta" if row.get("type") == 24 else "/v1"
        if prefix != expected_prefix:
            reasons.append("legacy_endpoint_path_contract_missing")
        # Empty legacy bases rely on compiled defaults. Do not guess a default.
        source_base = row.get("base_url")
        if type(source_base) is str and urlsplit(source_base).hostname == "gateway.ai.cloudflare.com":
            reasons.append("legacy_cloudflare_path_rewrite_not_supported")
        if (not endpoint_valid(source_base) or not endpoint_valid(provider.get("endpoint"))
                or prefix != expected_prefix or provider.get("endpoint") != source_base + expected_prefix):
            reasons.append("provider_endpoint_not_bound_to_source")
        if not text_reference(provider.get("contract_revision")):
            reasons.append("provider_contract_revision_missing")
    if credential is None:
        reasons.append("credential_reference_missing")
    else:
        require(credential.get("source_channel_sha256") == F.digest(row), "credential_source_channel_changed")
        require(type(key) is str and credential.get("source_key_sha256") == hashlib.sha256(key.encode()).hexdigest(),
                "credential_source_key_changed")
        reference = credential.get("reference")
        require(type(reference) is dict and set(reference) == {"id", "revision"}, "credential_reference_invalid")
        number(reference["id"], "credential_reference_invalid")
        number(reference["revision"], "credential_reference_invalid")
        path = credential.get("secret_file")
        require(type(path) is str and Path(path).is_absolute() and ".." not in path.split("/")
                and "." not in path.split("/") and not any(ord(c) < 32 for c in path), "credential_destination_invalid")
    return reasons


def text_reference(value):
    return type(value) is str and 1 <= len(value) <= 512 and not any(ord(c) < 32 for c in value)


def price_for(provider, group, model, options_hash):
    require(type(provider.get("prices")) is list and all(type(row) is dict for row in provider["prices"]), "price_contract_rows_invalid")
    rows = [row for row in provider.get("prices", []) if row.get("group") == group and row.get("model") == model]
    if len(rows) != 1:
        return None, "price_contract_missing_or_duplicate"
    price = rows[0]
    require(price.get("source_options_sha256") == options_hash, "price_source_options_changed")
    if price.get("kind") != "flat_tokens" or not text_reference(price.get("contract_revision")):
        return None, "legacy_expression_or_price_kind_not_supported"
    rates = price.get("rates")
    require(type(rates) is dict and set(rates) == {"input", "output", "cache_read", "cache_write", "request"}, "price_rates_invalid")
    for value in rates.values():
        number(value, "price_rates_invalid", 0)
    group_ppm = number(price.get("group_multiplier_ppm"), "price_multiplier_invalid", 0, 1_000_000_000)
    speed_ppm = number(price.get("speed_multiplier_ppm"), "price_multiplier_invalid", 1, 1_000_000_000)
    if rates["request"] or rates["cache_write"]:
        return None, "flat_price_dimension_not_supported_by_gateway"
    if any(rate * group_ppm * speed_ppm % 2_000_000_000_000
           or rate * group_ppm * speed_ppm // 2_000_000_000_000 > MAX_ID for rate in rates.values()):
        return None, "price_not_exact_integer_credits"
    return {"rates": rates, "group_multiplier_ppm": group_ppm, "speed_multiplier_ppm": speed_ppm}, None


def plan(tables, identities, bindings, *, runtime=None, providers=None, credentials=None, source_counts=None):
    require(all(table in tables for table in ("users", "channels", "abilities", "options")), "routing_source_tables_missing")
    users = unique(tables["users"], "id", "routing_source_user_duplicate")
    channels = unique(tables["channels"], "id", "routing_source_channel_duplicate")
    require(set(users) == set(identities), "routing_identity_user_coverage_mismatch")
    runtime = contract(runtime, "legacy-routing-runtime-evidence", bindings)
    providers = contract(providers, "legacy-routing-provider-contracts", bindings)
    credentials = contract(credentials, "legacy-routing-credential-references", bindings)
    provider_map = unique(providers["channels"], "channel_id", "provider_channel_duplicate") if providers else {}
    credential_map = unique(credentials["credentials"], "channel_id", "credential_channel_duplicate") if credentials else {}
    require(set(provider_map) <= set(channels) and set(credential_map) <= set(channels), "routing_orphan_channel_contract")
    blockers = []

    def block(code, **entity):
        blockers.append({"reason": code, **entity})

    if (type(source_counts) is not dict or type(source_counts.get("public_relay_preferences")) is not int
            or source_counts["public_relay_preferences"] != 0):
        block("legacy_user_channel_preferences_not_mapped")
    limits = None
    if runtime is None:
        block("runtime_evidence_missing")
    else:
        require(type(runtime.get("memory_cache_enabled")) is bool, "runtime_selection_mode_invalid")
        if runtime.get("streaming_enabled") is None:
            block("runtime_streaming_evidence_missing")
        else:
            require(type(runtime["streaming_enabled"]) is bool, "runtime_streaming_mode_invalid")
        require(type(runtime.get("source_revision")) is str and re.fullmatch(r"[a-f0-9]{40}", runtime["source_revision"])
                and text_reference(runtime.get("evidence_reference")), "runtime_source_revision_or_evidence_missing")
        if runtime.get("model_matching") != "exact":
            block("legacy_model_normalization_not_mapped")
        if runtime.get("max_attempts") is None:
            block("runtime_attempts_evidence_missing")
        elif number(runtime["max_attempts"], "runtime_attempts_invalid", 1, 1000) != 1:
            block("legacy_retry_semantics_not_mapped")
        limits = runtime.get("limits")
        if limits is None:
            block("runtime_native_limits_not_supplied")
        else:
            require(type(limits) is dict and set(limits) == {"input_token_ceiling", "lease_seconds", "timeout_seconds", "max_output_tokens"},
                    "runtime_limits_invalid")
            number(limits["input_token_ceiling"], "runtime_limits_invalid", 1, 16_000_000)
            number(limits["lease_seconds"], "runtime_limits_invalid", 1, 300)
            number(limits["timeout_seconds"], "runtime_limits_invalid", 1, 240)
            number(limits["max_output_tokens"], "runtime_limits_invalid", 1, 65_536)
            require(limits["timeout_seconds"] + 12 < limits["lease_seconds"], "runtime_lease_too_short")
    # Arbitrary old billing expressions are never replaced by supplied flat rates.
    option_keys = set()
    require(type(tables["options"]) is list, "routing_source_options_invalid")
    for option in tables["options"]:
        require(type(option) is dict and type(option.get("key")) is str and option["key"] not in option_keys,
                "routing_option_key_duplicate_or_invalid")
        option_keys.add(option["key"])
        if re.search(r"expr|expression", option["key"], re.I) and option.get("value") not in EMPTY + ("[]",):
            block("legacy_expression_pricing_not_compiled")
        if re.search(r"usablegroup|special.*group|groupgroupratio", option["key"], re.I) and option.get("value") not in EMPTY + ("[]",):
            block("cross_group_policy_not_mapped")
    options_hash = F.digest(tables["options"])
    memberships = []
    for uid, user in sorted(users.items()):
        group = user.get("group")
        require(label(group), "source_user_group_invalid")
        require(type(user.get("status")) is int and user["status"] in (1, 2) and "deleted_at" in user, "source_user_state_missing")
        if user["deleted_at"] is not None:
            require(type(user["deleted_at"]) is str and 10 <= len(user["deleted_at"]) <= 40, "source_user_deleted_at_invalid")
            datetime.fromisoformat(user["deleted_at"].replace("Z", "+00:00"))
        memberships.append({"user_id": uid, "group_name": group,
                            "enabled": user["status"] == 1 and user["deleted_at"] is None,
                            "source_reference": "legacy.users/" + str(uid), "source_row_sha256": F.digest(user)})
    ability_groups, seen = defaultdict(list), set()
    assessments = []
    require(type(tables["abilities"]) is list, "routing_source_abilities_invalid")
    for ability in tables["abilities"]:
        require(type(ability) is dict and type(ability.get("enabled")) is bool, "source_ability_invalid")
        group, model = ability.get("group"), ability.get("model")
        cid = number(ability.get("channel_id"), "source_ability_channel_invalid")
        require(label(group) and model_name(model), "source_ability_group_or_model_invalid")
        key = (group, model, cid)
        require(key not in seen and cid in channels, "source_ability_duplicate_or_orphan")
        seen.add(key)
        entry = {"group": group, "model": model, "channel_id": cid, "source_row_sha256": F.digest(ability),
                 "source_enabled": ability["enabled"], "reasons": []}
        assessments.append(entry)
        if ability["enabled"]:
            number(ability.get("priority"), "source_priority_missing_or_invalid", -(2**63))
            if channels[cid].get("status") != 1:
                entry["reasons"].append("ability_channel_status_disagrees")
            ability_groups[(group, model)].append((ability, entry))
    if runtime and runtime["memory_cache_enabled"]:
        projected = set()
        for cid, channel in channels.items():
            if channel.get("status") != 1:
                continue
            if type(channel.get("group")) is not str or type(channel.get("models")) is not str:
                block("cache_channel_projection_missing", channel_id=cid)
                continue
            projected.update((group, model, cid) for group in channel["group"].split(",") for model in channel["models"].split(","))
        enabled = {(group, model, ability["channel_id"]) for (group, model), pairs in ability_groups.items() for ability, _ in pairs}
        if projected != enabled:
            block("cache_channel_ability_projection_disagrees")
        for pairs in ability_groups.values():
            for ability, entry in pairs:
                channel = channels[ability["channel_id"]]
                if (channel.get("priority") != ability.get("priority") or type(channel.get("priority")) is not int
                        or channel.get("weight") != ability.get("weight") or type(channel.get("weight")) is not int):
                    entry["reasons"].append("cache_channel_priority_or_weight_disagrees")
    upstreams, secret_refs, external_models, routes, prices = {}, {}, {}, [], []
    reference_keys, destination_keys = {}, {}
    next_target_id = 0
    for (group, model), pairs in sorted(ability_groups.items()):
        priorities = sorted({a["priority"] for a, _ in pairs}, reverse=True)
        require(len(priorities) <= 65536, "native_priority_levels_exceeded")
        if len(pairs) > 64:
            block("native_route_target_limit_exceeded", group=group, model=model)
        targets, route_price = [], None
        for priority in priorities:
            tier = [a for a, _ in pairs if a["priority"] == priority]
            weights, weight_error = effective_weights(tier, runtime["memory_cache_enabled"]) if runtime else ({}, "runtime_evidence_missing")
            for ability, entry in pairs:
                if ability["priority"] != priority:
                    continue
                cid, channel = ability["channel_id"], channels[ability["channel_id"]]
                entry.update(native_priority=priorities.index(priority), native_weight=weights.get(cid))
                if weight_error:
                    entry["reasons"].append(weight_error)
                provider, credential = provider_map.get(cid), credential_map.get(cid)
                entry["reasons"].extend(blocked_channel(channel, provider, credential))
                if provider is not None:
                    require(type(provider.get("models")) is list and all(type(c) is dict for c in provider["models"]),
                            "provider_model_context_rows_invalid")
                    contexts = [c for c in provider.get("models", []) if c.get("source_model") == model]
                    if len(contexts) != 1:
                        entry["reasons"].append("provider_model_context_missing_or_duplicate")
                    else:
                        context = contexts[0]
                        allowed_operations = {"openai_chat": {"chat.completions"}, "openai_responses": {"responses"},
                                              "anthropic_messages": {"messages"}, "gemini": {"generateContent", "streamGenerateContent"}}
                        operations = context.get("source_operations")
                        if (type(operations) is not list or not operations or any(type(op) is not str for op in operations)
                                or not set(operations) <= allowed_operations.get(provider.get("protocol"), set())
                                or len(set(operations)) != len(operations)):
                            entry["reasons"].append("legacy_operation_surface_not_supported_or_unproven")
                        if provider.get("protocol") == "gemini" and context.get("legacy_api_version") != "v1beta":
                            entry["reasons"].append("legacy_gemini_version_contract_missing_or_unsupported")
                        if context.get("upstream_model") != model:
                            entry["reasons"].append("model_mapping_not_supported")
                        number(context.get("max_input_tokens"), "provider_input_bound_invalid", 1, 16_000_000)
                        number(context.get("max_output_tokens"), "provider_output_bound_invalid", 1, 65_536)
                        require(all(type(context.get(field)) is bool for field in ("missing_cached_tokens_is_zero", "missing_cache_creation_tokens_is_zero")),
                                "provider_usage_absence_contract_missing")
                        if limits and (context["max_input_tokens"] > limits["input_token_ceiling"]
                                       or limits["max_output_tokens"] > context["max_output_tokens"]):
                            entry["reasons"].append("provider_context_exceeds_runtime_limits")
                        external_models[(cid, model)] = {"upstream_id": cid, "upstream_model": model, "protocol": provider.get("protocol"),
                            "endpoint": provider.get("endpoint"), "contract_revision": provider.get("contract_revision"),
                            **{k: context[k] for k in ("max_input_tokens", "max_output_tokens", "missing_cached_tokens_is_zero", "missing_cache_creation_tokens_is_zero")}}
                    price, price_error = price_for(provider, group, model, options_hash)
                    if price_error:
                        entry["reasons"].append(price_error)
                    elif route_price is None:
                        route_price = price
                    elif F.digest(route_price) != F.digest(price):
                        entry["reasons"].append("channel_specific_prices_not_representable")
                if not entry["reasons"] and limits is not None:
                    upstreams[cid] = {"id": cid, "endpoint": provider["endpoint"], "protocol": provider["protocol"],
                                      "credential": credential["reference"], "enabled": True}
                    ref = (credential["reference"]["id"], credential["reference"]["revision"])
                    key_digest = credential["source_key_sha256"]
                    require(reference_keys.get(ref, key_digest) == key_digest
                            and destination_keys.get(credential["secret_file"], key_digest) == key_digest,
                            "credential_reference_or_destination_key_conflict")
                    reference_keys[ref] = key_digest
                    destination_keys[credential["secret_file"]] = key_digest
                    if ref in secret_refs:
                        require(secret_refs[ref] == {"reference": credential["reference"], "secret_file": credential["secret_file"]},
                                "credential_reference_destination_conflict")
                    secret_refs[ref] = {"reference": credential["reference"], "secret_file": credential["secret_file"]}
                    next_target_id += 1
                    targets.append({"id": next_target_id, "upstream_id": cid, "upstream_model": model, "enabled": True,
                                    "priority": entry["native_priority"], "weight": entry["native_weight"], "speeds": ["standard"],
                                    "payload": "adapt", "timeout_ms": limits["timeout_seconds"] * 1000})
                for reason in sorted(set(entry["reasons"])):
                    block(reason, group=group, model=model, channel_id=cid)
        route_id = len(routes) + 1
        routes.append({"id": route_id, "group": group, "model": model, "enabled": True, "speeds": ["standard"], "targets": targets,
                       "failover": {"max_attempts": 1, "retry_on": [], "allow_replay_after_send": False}})
        if route_price:
            prices.append({"id": route_id, "group": group, "model": model, "speed": "standard", **route_price})
    if not ability_groups:
        block("no_enabled_source_abilities")
    result = {"format_version": 1, "scope": "legacy-routing-offline-plan", "bindings": bindings,
              "status": "blocked" if blockers else "candidate-requires-native-validation", "apply_supported": False,
              "approval": False, "source_queries_this_run": 0, "source_options_sha256": options_hash,
              "input_contracts_sha256": {"runtime": F.digest(runtime), "providers": F.digest(providers), "credentials": F.digest(credentials)},
              "source_table_counts_sha256": F.digest(source_counts),
              "user_group_plan": memberships, "ability_assessments": assessments, "blockers": blockers,
              "unverified": ["supplied-contract-truth", "price-equivalence", "source-runtime-evidence-truth", "native-config-validation",
                             "legacy-operation-surface", "parameter-semantics-equivalence",
                             "secret-installation", "native-key-authority-and-settings", "wallet-and-provider-reconciliation", "target-state"],
              "counts": {"source_users": len(users), "planned_user_groups": len(memberships), "source_abilities": len(seen),
                         "enabled_source_abilities": sum(a["source_enabled"] for a in assessments), "blockers": len(blockers)}}
    if not blockers:
        result["candidate"] = {"version": 1, "routing": {"version": 1,
            "groups": [{"name": g, "enabled": True} for g in sorted({g for g, _ in ability_groups})],
            "upstreams": list(upstreams.values()), "routes": routes, "aliases": []}, "prices": {"version": 1, "rules": prices},
            "credentials": list(secret_refs.values()), "key_groups": [], "external_models": list(external_models.values()),
            "local_streaming": runtime["streaming_enabled"], **limits}
    result["plan_sha256"] = F.digest(result)
    return result


def toml_value(value):
    if type(value) is str:
        return json.dumps(value, ensure_ascii=False)
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int:
        return str(value)
    if type(value) is list:
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    require(type(value) is dict, "candidate_toml_type_invalid")
    return "{ " + ", ".join(json.dumps(key) + " = " + toml_value(item) for key, item in value.items()) + " }"


def public_summary(result):
    return {"status": result["status"], "apply_supported": False, "approval": False,
            "counts": result["counts"], "reasons": dict(Counter(row["reason"] for row in result["blockers"]))}


def verify_files(hashes):
    require(all(F.IDENTITY.sha256_file(path) == digest for path, digest in hashes.items()), "routing_input_changed_during_plan")


def bind_runtime_observation(runtime, bindings, hashes):
    """Bind optional observed process evidence; this does not approve activation."""
    if runtime is None or "evidence_file" not in runtime:
        return
    path = F.IDENTITY.private_backup_path(Path(runtime["evidence_file"]))
    observed_hash = F.IDENTITY.sha256_file(path)
    require(observed_hash == runtime.get("evidence_file_sha256"), "runtime_observation_file_changed")
    observed = F.parse_json(path.read_text())
    require(type(observed) is dict and observed.get("source_archive_sha256") == bindings["source_archive_sha256"]
            and observed.get("source_revision") == runtime.get("source_revision")
            and type(observed.get("memory_cache_enabled")) is bool
            and observed["memory_cache_enabled"] is runtime.get("memory_cache_enabled"), "runtime_observation_source_mismatch")
    hashes[path] = observed_hash


def load_routing_export(receipt, proof_path, hashes, *, retained_tables=("users", "channels", "abilities", "options")):
    """Validate the existing full export while retaining only four small tables.

    The financial reuser materializes every financial row. Use its same catalog,
    framing, instance and digest contracts without keeping unrelated large tables
    in memory. No new export, source query or weaker per-table coverage is used.
    """
    require(receipt.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified", "source_receipt_status_invalid")
    proof_path = F.IDENTITY.private_backup_path(proof_path)
    raw = F.IDENTITY.private_backup_path(proof_path.with_name("source-financial-export.private.jsonl"))
    archive = F.IDENTITY.private_backup_path(Path(receipt["archive"]["path"]))
    proof = F.parse_json(proof_path.read_text())
    require(F.IDENTITY.sha256_file(archive) == receipt["archive"]["sha256"] == proof.get("source_archive_sha256"), "source_archive_hash_mismatch")
    require(F.IDENTITY.sha256_file(raw) == proof.get("source_export_sha256"), "source_export_hash_mismatch")
    columns, baseline, metadata_hashes, _, _ = F.metadata(receipt)
    require(proof.get("metadata_hashes") == metadata_hashes and proof.get("source_column_coverage_exact") is True
            and proof.get("source_row_baseline_exact") is True, "source_export_baseline_unproven")
    selected = {table for table in columns if F.table_families(table) or F.FINANCIAL_NAME.search(table)}
    expected = {table: baseline[table] for table in selected}
    require(F.digest(proof.get("selected_table_counts")) == F.digest(expected), "source_export_table_selection_mismatch")
    require(type(retained_tables) is tuple and retained_tables and all(type(table) is str for table in retained_tables)
            and len(set(retained_tables)) == len(retained_tables), "retained_source_tables_invalid")
    tables = {table: [] for table in retained_tables}
    require(set(tables) <= selected, "routing_source_tables_missing")
    counts, identities, schemas = Counter(), 0, 0
    for name in receipt.get("private_metadata_files", []):
        path = F.IDENTITY.private_backup_path(Path(name))
        if path.name in metadata_hashes:
            hashes[path] = metadata_hashes[path.name]
    hashes[archive], hashes[raw] = receipt["archive"]["sha256"], proof["source_export_sha256"]
    with raw.open() as stream:
        for line in stream:
            tagged = F.parse_json(line)
            require(type(tagged) is dict, "source_export_row_shape_invalid")
            kind, row = tagged.get("kind"), tagged.get("row")
            if kind == "identity":
                identities += 1
                F.IDENTITY.verify_instance(row, receipt["readonly_reader"], receipt.get("source_instance_identity", {}), source=True)
                require(F.digest(row) == F.digest(proof["source_instance"]), "source_instance_changed_during_export")
            elif kind == "schema":
                schemas += 1
                require(type(row) is dict and row.get("schema") == receipt["source"]["schema"]
                        and row.get("schema_owner") == receipt["source_instance_identity"].get("schema_owner"), "source_schema_owner_mismatch")
            else:
                table = tagged.get("table")
                require(kind == "row" and table in selected and type(row) is dict and set(row) == columns[table], "source_column_coverage_mismatch")
                counts[table] += 1
                require(counts[table] <= baseline[table], "source_snapshot_row_baseline_mismatch")
                if table in tables:
                    tables[table].append(row)
    require(identities == schemas == 1, "source_export_identity_framing_invalid")
    require(all(counts[table] == baseline[table] for table in selected), "source_snapshot_row_baseline_mismatch")
    verify_files(hashes)
    return tables, proof, baseline


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ("source-receipt", "export-proof", "identity-map", "identity-manifest", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("runtime-evidence", "provider-contracts", "credential-refs"):
        parser.add_argument("--" + name, type=Path)
    args = parser.parse_args()
    try:
        input_roles = {name: value for name, value in vars(args).items() if name != "output" and value is not None}
        paths = [F.IDENTITY.private_backup_path(path) for path in input_roles.values()]
        hashes = {path: F.IDENTITY.sha256_file(path) for path in paths}
        tools = {Path(__file__): F.IDENTITY.sha256_file(Path(__file__)), Path(F.__file__): F.IDENTITY.sha256_file(Path(F.__file__)),
                 Path(F.IDENTITY.__file__): F.IDENTITY.sha256_file(Path(F.IDENTITY.__file__))}
        read = lambda path: F.parse_json(path.read_text()) if path else None
        receipt, manifest = read(args.source_receipt), read(args.identity_manifest)
        map_hash = hashes[args.identity_map]
        provenance = F.bind_identity_provenance(manifest, map_file_sha256=map_hash, source_archive_sha256=receipt["archive"]["sha256"])
        identities = F.identity_index([F.parse_json(line) for line in args.identity_map.read_text().splitlines() if line])
        tables, proof, source_counts = load_routing_export(receipt, args.export_proof, hashes)
        bindings = {"source_archive_sha256": receipt["archive"]["sha256"], "source_export_sha256": proof["source_export_sha256"],
                    "identity_map_file_sha256": map_hash}
        runtime = read(args.runtime_evidence)
        bind_runtime_observation(runtime, bindings, hashes)
        result = plan(tables, identities, bindings, runtime=runtime,
                      providers=read(args.provider_contracts), credentials=read(args.credential_refs), source_counts=source_counts)
        result.update(identity_provenance_sha256=F.digest(provenance), input_file_sha256={role: hashes[path] for role, path in input_roles.items()},
                      tool_source_sha256={path.name: sha for path, sha in tools.items()})
        if runtime is not None and "evidence_file" in runtime:
            result["input_file_sha256"]["runtime_observation"] = hashes[Path(runtime["evidence_file"])]
        result.pop("plan_sha256")
        result["plan_sha256"] = F.digest(result)
        verify_files(hashes); verify_files(tools)
        F.IDENTITY.private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        F.IDENTITY.private_backup_path(args.output, directory=True)
        F.IDENTITY.write_private(args.output, "routing-plan.private.json", F.canonical(result) + "\n")
        if "candidate" in result:
            candidate = "\n".join(json.dumps(key) + " = " + toml_value(value) for key, value in result["candidate"].items()) + "\n"
            require(len(candidate.encode()) <= 16 * 1024 * 1024, "candidate_toml_too_large")
            F.IDENTITY.write_private(args.output, "model-candidate.private.toml", candidate)
        verify_files(hashes); verify_files(tools)
        print(F.canonical(public_summary(result)))
        return 0
    except (MappingError, OSError, ValueError, KeyError, TypeError):
        # Exceptions may contain credential values, source records or paths.
        print('{"status":"failed","reason":"routing_inputs_or_private_output_invalid","apply_supported":false}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
