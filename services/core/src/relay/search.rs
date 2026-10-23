//! Hosted search uses a frozen supplier receipt contract, never a function name
//! or a requested search flag. Protocol references: QuantumNous/new-api
//! acb347a197ff0e22f651be9813b4a1a867342941 and d10f11ffff20688b8e3e745268cc0036e896d068
//! (AGPL-3.0); this is a native Rust implementation. See the official OpenAI
//! web-search and Google Search grounding guides for the measured dimensions.
//! https://developers.openai.com/api/docs/guides/tools-web-search
//! https://ai.google.dev/gemini-api/docs/google-search
use super::{ClientProtocol, Limits, RelayError, UpstreamProtocol};
use crate::routing::{SearchLimits, SearchProvider, SearchUsage};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SEARCH_UNITS: u64 = 10_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SearchBinding {
    pub provider: SearchProvider,
    pub limits: SearchLimits,
}

#[derive(Clone, Debug)]
pub(crate) enum HostedSearch {
    Responses {
        tools: Vec<Value>,
        max_tool_calls: Option<u64>,
        choice: Option<String>,
    },
    Gemini(Value),
    Anthropic(Value),
    Zhipu(Value),
}

pub(crate) fn provider_protocol(provider: SearchProvider) -> Option<UpstreamProtocol> {
    Some(match provider {
        SearchProvider::OpenAiResponsesV1
        | SearchProvider::AzureResponsesV1
        | SearchProvider::XaiResponsesV1
        | SearchProvider::DashscopeResponsesV1 => UpstreamProtocol::Responses,
        SearchProvider::GeminiGroundingV1 => UpstreamProtocol::Gemini,
        SearchProvider::AnthropicMessagesV1 | SearchProvider::DashscopeAnthropicV1 => {
            UpstreamProtocol::Anthropic
        }
        SearchProvider::ZhipuChatV1 => UpstreamProtocol::Chat,
        SearchProvider::DeepSeekSearchV1 | SearchProvider::DeepSeekFetchV1 => return None,
    })
}

impl HostedSearch {
    pub(crate) fn unbound(&self, value: &mut Value) -> Result<(), RelayError> {
        match self {
            Self::Responses {
                tools,
                max_tool_calls,
                choice,
            } => {
                append_tools(value, tools.iter().cloned())?;
                if let Some(cap) = max_tool_calls {
                    value["max_tool_calls"] = json!(cap);
                }
                if let Some(choice) = choice {
                    value["tool_choice"] = json!({"type":choice});
                }
            }
            Self::Gemini(tool) => append_tools(value, [json!({"googleSearch":tool})])?,
            Self::Anthropic(tool) | Self::Zhipu(tool) => append_tools(value, [tool.clone()])?,
        }
        Ok(())
    }
    pub(crate) fn validate_binding(
        &self,
        client: ClientProtocol,
        protocol: UpstreamProtocol,
        binding: SearchBinding,
        stream: bool,
    ) -> Result<(), RelayError> {
        binding
            .limits
            .validate()
            .map_err(|_| RelayError::Invalid("invalid search bounds"))?;
        if provider_protocol(binding.provider) != Some(protocol) {
            return Err(RelayError::Unsupported(
                "search receipt contract does not match protocol",
            ));
        }
        let same_protocol = matches!(
            (client, protocol),
            (ClientProtocol::Chat, UpstreamProtocol::Chat)
                | (ClientProtocol::Responses, UpstreamProtocol::Responses)
                | (ClientProtocol::Anthropic, UpstreamProtocol::Anthropic)
                | (ClientProtocol::Gemini, UpstreamProtocol::Gemini)
        );
        if !same_protocol {
            return Err(RelayError::Unsupported(
                "hosted search requires a native protocol route",
            ));
        }
        match self {
            Self::Responses {
                tools,
                max_tool_calls,
                ..
            } => {
                if protocol != UpstreamProtocol::Responses {
                    return Err(RelayError::Unsupported(
                        "Responses search requires Responses",
                    ));
                }
                let web = tools.iter().any(|tool| tool["type"] == "web_search");
                let x = tools.iter().any(|tool| tool["type"] == "x_search");
                if x && binding.provider != SearchProvider::XaiResponsesV1 {
                    return Err(RelayError::Unsupported(
                        "x_search requires its reviewed xAI contract",
                    ));
                }
                if (web && binding.limits.calls.is_none_or(|n| n == 0))
                    || (x
                        && (binding.limits.x_posts_fetched.is_none()
                            || binding.limits.x_users_fetched.is_none()))
                {
                    return Err(RelayError::Unsupported(
                        "search requires supplier-backed bounds",
                    ));
                }
                if binding.provider != SearchProvider::OpenAiResponsesV1
                    && (max_tool_calls.is_some()
                        || tools
                            .iter()
                            .any(|tool| tool.as_object().is_some_and(|object| object.len() != 1)))
                {
                    return Err(RelayError::Unsupported(
                        "supplier search cannot preserve these OpenAI controls",
                    ));
                }
                if max_tool_calls.is_some_and(|n| binding.limits.calls.is_some_and(|cap| n > cap)) {
                    return Err(RelayError::Invalid(
                        "requested search limit exceeds supplier contract",
                    ));
                }
            }
            Self::Gemini(_) => {
                if protocol != UpstreamProtocol::Gemini
                    || (binding.limits.queries.is_none()
                        && binding.limits.grounded_requests.is_none())
                {
                    return Err(RelayError::Unsupported(
                        "Google Search requires reviewed grounding bounds",
                    ));
                }
            }
            Self::Anthropic(tool) => {
                let cap = binding.limits.calls.ok_or(RelayError::Unsupported(
                    "search requires supplier-backed bounds",
                ))?;
                if cap == 0 {
                    return Err(RelayError::Unsupported("search bound must be positive"));
                }
                if protocol != UpstreamProtocol::Anthropic {
                    return Err(RelayError::Unsupported(
                        "Anthropic search requires Messages",
                    ));
                }
                if tool
                    .get("max_uses")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > cap)
                {
                    return Err(RelayError::Invalid(
                        "requested search limit exceeds supplier contract",
                    ));
                }
            }
            Self::Zhipu(_) => {
                if protocol != UpstreamProtocol::Chat
                    || binding.limits.grounded_requests != Some(1)
                    || stream
                {
                    return Err(RelayError::Unsupported(
                        "Zhipu search needs a non-stream grounded-request contract",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn apply(
        &self,
        value: &mut Value,
        binding: SearchBinding,
    ) -> Result<(), RelayError> {
        match self {
            Self::Responses {
                tools,
                max_tool_calls,
                choice,
            } => {
                append_tools(value, tools.iter().cloned())?;
                if binding.provider == SearchProvider::OpenAiResponsesV1
                    && let Some(cap) = binding.limits.calls
                {
                    value["max_tool_calls"] = json!(max_tool_calls.unwrap_or(cap));
                }
                if let Some(choice) = choice {
                    value["tool_choice"] = json!({"type": choice});
                }
            }
            Self::Gemini(tool) => append_tools(value, [json!({"googleSearch": tool})])?,
            Self::Anthropic(tool) => {
                let mut tool = tool.clone();
                if tool.get("max_uses").is_none() {
                    tool["max_uses"] = json!(
                        binding
                            .limits
                            .calls
                            .ok_or(RelayError::Unsupported("unbounded search"))?
                    );
                }
                append_tools(value, [tool])?;
                if binding.provider == SearchProvider::DashscopeAnthropicV1 {
                    // DashScope requires the marker in a system text block.
                    let marker = json!({"type":"text","text":"x-anthropic-billing-header: cc_entrypoint=cli;"});
                    let system = value
                        .as_object_mut()
                        .ok_or(RelayError::Invalid("invalid upstream request"))?
                        .remove("system");
                    let mut blocks = match system {
                        None | Some(Value::Null) => Vec::new(),
                        Some(Value::String(text)) => vec![json!({"type":"text","text":text})],
                        Some(Value::Array(blocks)) => blocks,
                        _ => return Err(RelayError::Invalid("invalid system blocks")),
                    };
                    if !blocks.iter().any(|block| {
                        block["text"]
                            .as_str()
                            .is_some_and(|text| text.contains("cc_entrypoint=cli"))
                    }) {
                        blocks.insert(0, marker);
                    }
                    value["system"] = json!(blocks);
                }
            }
            Self::Zhipu(tool) => append_tools(value, [tool.clone()])?,
        }
        Ok(())
    }
}

fn append_tools(
    value: &mut Value,
    tools: impl IntoIterator<Item = Value>,
) -> Result<(), RelayError> {
    if value.get("tools").is_none() {
        value["tools"] = json!([]);
    }
    value["tools"]
        .as_array_mut()
        .ok_or(RelayError::Invalid("invalid upstream tools"))?
        .extend(tools);
    Ok(())
}

pub(crate) fn keys(value: &Value, allowed: &[&str]) -> Result<(), RelayError> {
    let object = value
        .as_object()
        .ok_or(RelayError::Invalid("invalid hosted search object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(RelayError::Unsupported("unsupported hosted search option"));
    }
    Ok(())
}

pub(crate) fn responses_tool(value: &Value) -> Result<Value, RelayError> {
    keys(
        value,
        &[
            "type",
            "search_context_size",
            "filters",
            "user_location",
            "external_web_access",
        ],
    )?;
    if !matches!(value["type"].as_str(), Some("web_search" | "x_search")) {
        return Err(RelayError::Unsupported("unsupported hosted search tool"));
    }
    if value["type"] == "x_search" && value.as_object().is_some_and(|object| object.len() != 1) {
        return Err(RelayError::Unsupported(
            "x_search options are not supported",
        ));
    }
    if value
        .get("search_context_size")
        .is_some_and(|v| !matches!(v.as_str(), Some("low" | "medium" | "high")))
    {
        return Err(RelayError::Invalid("invalid search context size"));
    }
    if let Some(filters) = value.get("filters") {
        keys(filters, &["allowed_domains", "blocked_domains"])?;
        for key in ["allowed_domains", "blocked_domains"] {
            if let Some(domains) = filters.get(key) {
                let domains = domains
                    .as_array()
                    .ok_or(RelayError::Invalid("invalid domain filter"))?;
                if domains.len() > 100
                    || domains.iter().any(|domain| {
                        domain.as_str().is_none_or(|s| {
                            s.is_empty() || s.len() > 253 || s.chars().any(char::is_whitespace)
                        })
                    })
                {
                    return Err(RelayError::Invalid("invalid domain filter"));
                }
            }
        }
    }
    if let Some(location) = value.get("user_location") {
        keys(location, &["type", "city", "region", "country", "timezone"])?;
        if location["type"] != "approximate"
            || location
                .as_object()
                .is_some_and(|o| o.values().any(|v| v.as_str().is_none_or(|s| s.len() > 256)))
        {
            return Err(RelayError::Invalid("invalid search location"));
        }
    }
    if value
        .get("external_web_access")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(RelayError::Invalid("invalid external web access flag"));
    }
    Ok(value.clone())
}

pub(crate) fn anthropic_tool(value: &Value) -> Result<Value, RelayError> {
    keys(
        value,
        &[
            "type",
            "name",
            "max_uses",
            "allowed_domains",
            "blocked_domains",
            "user_location",
        ],
    )?;
    if value["type"] != "web_search_20250305" || value["name"] != "web_search" {
        return Err(RelayError::Unsupported(
            "unsupported Anthropic hosted search version",
        ));
    }
    if let Some(n) = value.get("max_uses") {
        bounded_count(n)?;
        if n.as_u64() == Some(0) {
            return Err(RelayError::Invalid("search limit must be positive"));
        }
    }
    let mut validation = json!({"type":"web_search"});
    for key in ["allowed_domains", "blocked_domains"] {
        if let Some(domains) = value.get(key) {
            if validation.get("filters").is_none() {
                validation["filters"] = json!({});
            }
            validation["filters"][key] = domains.clone();
        }
    }
    if let Some(location) = value.get("user_location") {
        validation["user_location"] = location.clone();
    }
    responses_tool(&validation)?;
    Ok(value.clone())
}

pub(crate) fn zhipu_tool(value: &Value) -> Result<Value, RelayError> {
    keys(value, &["type", "web_search"])?;
    let search = &value["web_search"];
    keys(
        search,
        &[
            "enable",
            "search_engine",
            "search_result",
            "search_intent",
            "search_domain_filter",
            "content_size",
        ],
    )?;
    if value["type"] != "web_search"
        || search["enable"] != true
        || search["search_engine"] != "search_std"
        || search["search_result"] != true
    {
        return Err(RelayError::Unsupported(
            "Zhipu search needs explicit search_std result evidence",
        ));
    }
    if search.get("search_intent").is_some_and(|v| !v.is_boolean())
        || search
            .get("content_size")
            .is_some_and(|v| !matches!(v.as_str(), Some("medium" | "high")))
        || search
            .get("search_domain_filter")
            .is_some_and(|v| v.as_str().is_none_or(|s| s.is_empty() || s.len() > 253))
    {
        return Err(RelayError::Invalid("invalid Zhipu search option"));
    }
    Ok(value.clone())
}

pub(crate) fn bounded_count(value: &Value) -> Result<u64, RelayError> {
    value
        .as_u64()
        .filter(|n| *n <= MAX_SEARCH_UNITS)
        .ok_or(RelayError::Protocol("invalid search count"))
}

fn path<'a>(value: &'a Value, names: &[&str]) -> Result<Option<&'a Value>, RelayError> {
    let mut at = value;
    for name in names {
        if at.is_null() {
            return Ok(None);
        }
        let object = at
            .as_object()
            .ok_or(RelayError::Protocol("invalid search receipt object"))?;
        let Some(next) = object.get(*name) else {
            return Ok(None);
        };
        at = next;
    }
    Ok((!at.is_null()).then_some(at))
}

fn count(value: &Value, names: &[&str]) -> Result<Option<u64>, RelayError> {
    path(value, names)?.map(bounded_count).transpose()
}

pub(crate) fn unknown_usage(provider: SearchProvider) -> SearchUsage {
    SearchUsage {
        provider,
        calls: None,
        queries: None,
        grounded_requests: None,
        x_posts_fetched: None,
        x_users_fetched: None,
    }
}

pub(crate) struct SearchMeter {
    binding: SearchBinding,
    observed: SearchUsage,
    terminal: Option<SearchUsage>,
    items: BTreeMap<String, (Option<String>, bool)>,
    queries: BTreeSet<String>,
    query_evidence: bool,
    grounded: bool,
    bytes: usize,
    byte_limit: usize,
}

impl SearchMeter {
    pub(crate) fn new(binding: SearchBinding, limits: &Limits) -> Self {
        Self {
            binding,
            observed: unknown_usage(binding.provider),
            terminal: None,
            items: BTreeMap::new(),
            queries: BTreeSet::new(),
            query_evidence: false,
            grounded: false,
            bytes: 0,
            byte_limit: limits.output_bytes,
        }
    }

    fn account(&mut self, bytes: usize) -> Result<(), RelayError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(RelayError::Limit("search evidence"))?;
        if self.bytes > self.byte_limit {
            return Err(RelayError::Limit("search evidence"));
        }
        Ok(())
    }

    pub(crate) fn item(&mut self, value: &Value) -> Result<(), RelayError> {
        if value["type"] != "web_search_call" && value["type"] != "x_search_call" {
            return Ok(());
        }
        if provider_protocol(self.binding.provider) != Some(UpstreamProtocol::Responses) {
            return Err(RelayError::Protocol(
                "search item disagrees with supplier contract",
            ));
        }
        if value["type"] == "x_search_call"
            && self.binding.provider != SearchProvider::XaiResponsesV1
        {
            return Err(RelayError::Protocol("unexpected x_search receipt"));
        }
        let id = value["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .ok_or(RelayError::Protocol("invalid hosted search ID"))?;
        let completed = match value["status"].as_str() {
            Some("completed") => true,
            Some("in_progress" | "searching") => false,
            _ => return Err(RelayError::Protocol("unknown hosted search status")),
        };
        let action = value
            .get("action")
            .filter(|v| !v.is_null())
            .map(|v| {
                let action = v
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or(RelayError::Protocol("invalid search action"))?;
                if !matches!(action, "search" | "open_page" | "find_in_page") {
                    return Err(RelayError::Protocol("unsupported hosted search action"));
                }
                Ok(action.to_owned())
            })
            .transpose()?;
        if let Some((old_action, old_completed)) = self.items.get(id) {
            if (*old_completed && !completed)
                || (old_action.is_some() && action.is_some() && *old_action != action)
            {
                return Err(RelayError::Protocol("hosted search identity changed"));
            }
            let merged = action.or_else(|| old_action.clone());
            self.items
                .insert(id.to_owned(), (merged, *old_completed || completed));
        } else {
            if self.items.len() >= MAX_SEARCH_UNITS as usize {
                return Err(RelayError::Limit("search item count"));
            }
            self.account(id.len())?;
            self.items.insert(id.to_owned(), (action, completed));
        }
        Ok(())
    }

    pub(crate) fn grounding(&mut self, metadata: &Value) -> Result<(), RelayError> {
        if self.binding.provider != SearchProvider::GeminiGroundingV1 || !metadata.is_object() {
            return Err(RelayError::Protocol("invalid grounding receipt"));
        }
        for key in ["webSearchQueries", "imageSearchQueries"] {
            if let Some(values) = metadata.get(key) {
                let values = values
                    .as_array()
                    .ok_or(RelayError::Protocol("invalid grounding queries"))?;
                self.query_evidence = true;
                for query in values {
                    let query = query
                        .as_str()
                        .ok_or(RelayError::Protocol("invalid grounding query"))?
                        .trim();
                    if query.is_empty() {
                        continue;
                    }
                    if query.len() > 4096 || query.chars().any(char::is_control) {
                        return Err(RelayError::Protocol("invalid grounding query"));
                    }
                    if !self.queries.contains(query) {
                        if self.queries.len() >= MAX_SEARCH_UNITS as usize {
                            return Err(RelayError::Limit("grounding query count"));
                        }
                        self.account(query.len())?;
                        self.queries.insert(query.to_owned());
                    }
                }
            }
        }
        if let Some(chunks) = metadata.get("groundingChunks") {
            let chunks = chunks
                .as_array()
                .ok_or(RelayError::Protocol("invalid grounding chunks"))?;
            self.grounded |= !chunks.is_empty();
        }
        self.grounded |= !self.queries.is_empty();
        Ok(())
    }

    pub(crate) fn observe(
        &mut self,
        value: &Value,
        terminal: bool,
    ) -> Result<SearchUsage, RelayError> {
        let mut next = unknown_usage(self.binding.provider);
        match self.binding.provider {
            SearchProvider::OpenAiResponsesV1 => {
                if let Some(output) = value.get("output") {
                    let output = output
                        .as_array()
                        .ok_or(RelayError::Protocol("invalid search output"))?;
                    if terminal
                        && self.items.keys().any(|id| {
                            !output.iter().any(|item| {
                                item["id"] == id.as_str()
                                    && matches!(
                                        item["type"].as_str(),
                                        Some("web_search_call" | "x_search_call")
                                    )
                            })
                        })
                    {
                        return Err(RelayError::Protocol(
                            "final search output lost a recorded item",
                        ));
                    }
                    for item in output {
                        self.item(item)?;
                    }
                } else if terminal {
                    return Err(RelayError::Protocol(
                        "terminal response lacks search inventory",
                    ));
                }
                let incomplete = self
                    .items
                    .values()
                    .any(|(action, done)| !*done || action.is_none());
                if !incomplete && (!self.items.is_empty() || value.get("output").is_some()) {
                    next.calls = Some(
                        self.items
                            .values()
                            .filter(|(action, done)| *done && action.as_deref() == Some("search"))
                            .count() as u64,
                    );
                }
            }
            SearchProvider::AzureResponsesV1 => {
                next.calls = count(value, &["tool_usage", "web_search", "num_requests"])?
            }
            SearchProvider::XaiResponsesV1 => {
                next.calls = count(
                    value,
                    &[
                        "usage",
                        "server_side_tool_usage_details",
                        "web_search_calls",
                    ],
                )?;
                next.x_posts_fetched = count(
                    value,
                    &["usage", "server_side_tool_usage_details", "x_posts_fetched"],
                )?;
                next.x_users_fetched = count(
                    value,
                    &["usage", "server_side_tool_usage_details", "x_users_fetched"],
                )?;
            }
            SearchProvider::DashscopeResponsesV1 => {
                next.calls = count(value, &["usage", "x_tools", "web_search", "count"])?
            }
            SearchProvider::AnthropicMessagesV1 | SearchProvider::DashscopeAnthropicV1 => {
                next.calls = count(value, &["usage", "server_tool_use", "web_search_requests"])?
            }
            SearchProvider::GeminiGroundingV1 => {
                if let Some(candidates) = value.get("candidates") {
                    for candidate in candidates
                        .as_array()
                        .ok_or(RelayError::Protocol("invalid grounding candidates"))?
                    {
                        if let Some(metadata) =
                            candidate.get("groundingMetadata").filter(|v| !v.is_null())
                        {
                            self.grounding(metadata)?;
                        }
                    }
                }
                if self.query_evidence {
                    next.queries = Some(self.queries.len() as u64);
                }
                if self.query_evidence || self.grounded {
                    next.grounded_requests = Some(u64::from(self.grounded));
                }
            }
            SearchProvider::ZhipuChatV1 => {
                if let Some(results) = value.get("web_search") {
                    let results = results
                        .as_array()
                        .ok_or(RelayError::Protocol("invalid Zhipu search results"))?;
                    // An empty result list is not proof that no search was used.
                    if !results.is_empty() {
                        next.grounded_requests = Some(1);
                    }
                }
            }
            SearchProvider::DeepSeekSearchV1 | SearchProvider::DeepSeekFetchV1 => {
                return Err(RelayError::Unsupported(
                    "native search tools require their own endpoint",
                ));
            }
        }
        if !next.valid() || !self.binding.limits.contains(next) {
            return Err(RelayError::Protocol("search usage exceeds frozen contract"));
        }
        if terminal {
            self.terminal = Some(next);
        }
        let mut combined = self.observed;
        merge_usage(&mut combined, next)?;
        self.observed = combined;
        Ok(combined)
    }

    pub(crate) fn complete(&mut self) -> Result<SearchUsage, RelayError> {
        let usage = if self.binding.provider == SearchProvider::GeminiGroundingV1 {
            self.observe(&Value::Null, true)?;
            self.terminal
        } else {
            self.terminal
        }
        .ok_or(RelayError::Protocol("missing final search receipt"))?;
        for (cap, measured) in [
            (self.binding.limits.calls, usage.calls),
            (self.binding.limits.queries, usage.queries),
            (
                self.binding.limits.grounded_requests,
                usage.grounded_requests,
            ),
            (self.binding.limits.x_posts_fetched, usage.x_posts_fetched),
            (self.binding.limits.x_users_fetched, usage.x_users_fetched),
        ] {
            if cap.is_some() && measured.is_none() {
                return Err(RelayError::Protocol("missing final search counter"));
            }
        }
        Ok(self.observed)
    }
}

pub(crate) fn merge_usage(previous: &mut SearchUsage, next: SearchUsage) -> Result<(), RelayError> {
    if previous.provider != next.provider || !next.valid() {
        return Err(RelayError::Protocol("search usage contract changed"));
    }
    let mut combined = *previous;
    for (old, new) in [
        (&mut combined.calls, next.calls),
        (&mut combined.queries, next.queries),
        (&mut combined.grounded_requests, next.grounded_requests),
        (&mut combined.x_posts_fetched, next.x_posts_fetched),
        (&mut combined.x_users_fetched, next.x_users_fetched),
    ] {
        if let Some(value) = new {
            if old.is_some_and(|old| value < old) {
                return Err(RelayError::Protocol("search usage decreased"));
            }
            *old = Some(value);
        }
    }
    if !combined.valid() || !combined.follows(*previous) {
        return Err(RelayError::Protocol("invalid cumulative search usage"));
    }
    *previous = combined;
    Ok(())
}

pub(crate) fn reject_uncontracted_usage(value: &Value) -> Result<(), RelayError> {
    for names in [
        &["tool_usage", "web_search", "num_requests"][..],
        &[
            "usage",
            "server_side_tool_usage_details",
            "web_search_calls",
        ][..],
        &["usage", "server_side_tool_usage_details", "x_posts_fetched"][..],
        &["usage", "server_side_tool_usage_details", "x_users_fetched"][..],
        &["usage", "x_tools", "web_search", "count"][..],
        &["usage", "server_tool_use", "web_search_requests"][..],
    ] {
        if count(value, names)?.is_some_and(|count| count != 0) {
            return Err(RelayError::Protocol("uncontracted search usage"));
        }
    }
    if let Some(results) = value.get("web_search") {
        let results = results
            .as_array()
            .ok_or(RelayError::Protocol("invalid search results"))?;
        if !results.is_empty() {
            return Err(RelayError::Protocol("uncontracted search results"));
        }
    }
    Ok(())
}

fn citation_url(value: &Value) -> Result<(), RelayError> {
    let text = value
        .as_str()
        .filter(|text| text.len() <= 8192 && !text.chars().any(char::is_control))
        .ok_or(RelayError::Protocol("invalid citation URL"))?;
    let url =
        reqwest::Url::parse(text).map_err(|_| RelayError::Protocol("invalid citation URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(RelayError::Protocol("invalid citation URL"));
    }
    Ok(())
}

pub(crate) fn response_citations(value: &Value) -> Result<Vec<Value>, RelayError> {
    let values = value
        .as_array()
        .ok_or(RelayError::Protocol("invalid search annotations"))?;
    if values.len() > MAX_SEARCH_UNITS as usize {
        return Err(RelayError::Limit("search annotations"));
    }
    for value in values {
        if value["type"] != "url_citation"
            || value["title"].as_str().is_none()
            || value["start_index"].as_u64().is_none()
            || value["end_index"].as_u64().is_none()
            || value["start_index"].as_u64() > value["end_index"].as_u64()
        {
            return Err(RelayError::Protocol("unsupported search annotation"));
        }
        citation_url(&value["url"])?;
    }
    Ok(values.clone())
}

pub(crate) fn anthropic_citations(value: &Value) -> Result<Vec<Value>, RelayError> {
    let values = value
        .as_array()
        .ok_or(RelayError::Protocol("invalid Anthropic citations"))?;
    if values.len() > MAX_SEARCH_UNITS as usize {
        return Err(RelayError::Limit("search citations"));
    }
    for value in values {
        if value["type"] != "web_search_result_location"
            || value["title"].as_str().is_none()
            || value["cited_text"].as_str().is_none()
            || value["encrypted_index"].as_str().is_none()
        {
            return Err(RelayError::Protocol(
                "unsupported Anthropic search citation",
            ));
        }
        citation_url(&value["url"])?;
    }
    Ok(values.clone())
}

pub(crate) fn anthropic_search_block(value: &Value) -> Result<(), RelayError> {
    match value["type"].as_str() {
        Some("server_tool_use") => {
            if value["name"] != "web_search" || !value["input"].is_object() {
                return Err(RelayError::Protocol("unsupported server tool"));
            }
            value["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or(RelayError::Protocol("invalid server tool ID"))?;
        }
        Some("web_search_tool_result") => {
            value["tool_use_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or(RelayError::Protocol("invalid search result ID"))?;
            match &value["content"] {
                Value::Array(results) => {
                    for result in results {
                        if result["type"] != "web_search_result"
                            || result["title"].as_str().is_none()
                            || result["encrypted_content"].as_str().is_none()
                        {
                            return Err(RelayError::Protocol("invalid search result"));
                        }
                        citation_url(&result["url"])?;
                    }
                }
                Value::Object(_)
                    if value["content"]["type"] == "web_search_tool_result_error"
                        && value["content"]["error_code"].is_string() => {}
                _ => return Err(RelayError::Protocol("invalid search result content")),
            }
        }
        _ => return Err(RelayError::Protocol("unsupported hosted search block")),
    }
    Ok(())
}

/// Completed tool items may gain optional fields in a later snapshot, but a
/// repeated ID cannot replace already observed arguments, sources or metadata.
pub(crate) fn merge_response_output(previous: &Value, next: &Value) -> Result<Value, RelayError> {
    if previous.is_null() {
        return Ok(next.clone());
    }
    if next.is_null() {
        return Ok(previous.clone());
    }
    if let (Some(old), Some(new)) = (previous.as_object(), next.as_object()) {
        let mut merged = old.clone();
        for (key, value) in new {
            let combined = if key == "status" {
                if old.get(key).is_some_and(|old| old == "completed") && value != "completed" {
                    return Err(RelayError::Protocol("hosted output status decreased"));
                }
                value.clone()
            } else if let Some(old) = old.get(key) {
                merge_response_output(old, value)?
            } else {
                value.clone()
            };
            merged.insert(key.clone(), combined);
        }
        return Ok(Value::Object(merged));
    }
    if previous != next {
        return Err(RelayError::Protocol("hosted output identity changed"));
    }
    Ok(previous.clone())
}

/// Accumulate grounding snapshots without dropping earlier sources or query
/// evidence. Chunk references in each new support use that frame's indices.
pub(crate) fn merge_grounding(
    previous: &mut Option<Value>,
    next: &Value,
) -> Result<(), RelayError> {
    let object = next
        .as_object()
        .ok_or(RelayError::Protocol("invalid grounding metadata"))?;
    let mut combined = previous.clone().unwrap_or_else(|| json!({}));
    let mut remap = Vec::new();
    if let Some(chunks) = object.get("groundingChunks") {
        let chunks = chunks
            .as_array()
            .ok_or(RelayError::Protocol("invalid grounding chunks"))?;
        if combined.get("groundingChunks").is_none() {
            combined["groundingChunks"] = json!([]);
        }
        let merged = combined["groundingChunks"]
            .as_array_mut()
            .ok_or(RelayError::Protocol("invalid grounding chunks"))?;
        for chunk in chunks {
            if !chunk.is_object() {
                return Err(RelayError::Protocol("invalid grounding chunk"));
            }
            let index = if let Some(index) = merged.iter().position(|old| old == chunk) {
                index
            } else {
                merged.push(chunk.clone());
                merged.len() - 1
            };
            if merged.len() > MAX_SEARCH_UNITS as usize {
                return Err(RelayError::Limit("grounding metadata"));
            }
            remap.push(index);
        }
    }
    for (key, value) in object {
        if key == "groundingChunks" {
            continue;
        }
        if let Value::Array(values) = value {
            if combined.get(key).is_none() {
                combined[key] = json!([]);
            }
            let current_chunks = combined
                .get("groundingChunks")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let merged = combined[key]
                .as_array_mut()
                .ok_or(RelayError::Protocol("grounding field changed type"))?;
            for original in values {
                let mut entry = original.clone();
                if key == "groundingSupports" {
                    if !entry.is_object() {
                        return Err(RelayError::Protocol("invalid grounding support"));
                    }
                    if let Some(indices) = entry.get_mut("groundingChunkIndices") {
                        for index in indices
                            .as_array_mut()
                            .ok_or(RelayError::Protocol("invalid grounding indices"))?
                        {
                            let original = index
                                .as_u64()
                                .and_then(|n| usize::try_from(n).ok())
                                .ok_or(RelayError::Protocol("invalid grounding index"))?;
                            let mapped = if object.contains_key("groundingChunks") {
                                *remap
                                    .get(original)
                                    .ok_or(RelayError::Protocol("orphan grounding source"))?
                            } else if original < current_chunks {
                                original
                            } else {
                                return Err(RelayError::Protocol("orphan grounding source"));
                            };
                            *index = json!(mapped);
                        }
                    }
                }
                if !merged.contains(&entry) {
                    merged.push(entry);
                }
                if merged.len() > MAX_SEARCH_UNITS as usize {
                    return Err(RelayError::Limit("grounding metadata"));
                }
            }
        } else {
            combined[key] = value.clone();
        }
    }
    *previous = Some(combined);
    Ok(())
}
