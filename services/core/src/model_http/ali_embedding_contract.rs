//! Bounded native Ali multimodal embeddings and the OpenAI embedding envelope.
//! Source: https://www.alibabacloud.com/help/en/model-studio/multimodal-embedding-api-reference
//! Upstream comparison: QuantumNous/new-api afe73b08ff6432aca4749a024391c7f886a5bd2d
//! (AGPL-3.0). Token meanings are selected by an exact host contract, not names.
use crate::{
    model_http::ExternalEmbeddingContract,
    relay::{RelayError, Usage, native_anthropic::unambiguous_json},
    routing::{EmbeddingProvider, EmbeddingUsage, MAX_EMBEDDING_INPUT_TOKENS},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(super) const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Client {
    OpenAi,
    Native,
}
pub(super) struct Request {
    pub client: Client,
    pub model: String,
    pub wire: Value,
    dimensions: Option<u32>,
    input_budget: Option<u64>,
    fusion: bool,
    items: Vec<Item>,
    instruction_bytes: usize,
}
enum Item {
    Text(usize),
    InlineImage(usize),
    RemoteImage,
    RemoteVideo,
}
pub(super) struct Bound {
    provider: EmbeddingProvider,
    dimensions: u32,
    count: usize,
    input_limit: u64,
}
pub(super) struct Observed {
    pub output: Value,
    pub usage: Usage,
}

impl Request {
    pub(super) fn parse(client: Client, bytes: &[u8]) -> Result<Self, RelayError> {
        if bytes.len() > MAX_BODY {
            return Err(RelayError::Limit("embedding request body"));
        }
        let value = unambiguous_json(bytes)
            .map_err(|_| RelayError::Invalid("invalid or ambiguous embedding JSON"))?;
        let model = text(&value["model"])?;
        if model.is_empty()
            || model.len() > 200
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/:@~".contains(&b))
        {
            return Err(RelayError::Invalid("invalid embedding model"));
        }
        let input_budget = value
            .get("max_input_tokens")
            .map(|v| {
                v.as_u64()
                    .filter(|v| (1..=MAX_EMBEDDING_INPUT_TOKENS).contains(v))
                    .ok_or(RelayError::Invalid("invalid embedding input budget"))
            })
            .transpose()?;
        let (contents, mut parameters) = match client {
            Client::Native => {
                object(
                    &value,
                    &["model", "input", "parameters", "max_input_tokens"],
                )?;
                object(&value["input"], &["contents"])?;
                let contents = value["input"]["contents"]
                    .as_array()
                    .ok_or(RelayError::Invalid("embedding contents must be an array"))?
                    .clone();
                let parameters = value
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                (contents, parameters)
            }
            Client::OpenAi => {
                object(
                    &value,
                    &[
                        "model",
                        "input",
                        "dimensions",
                        "encoding_format",
                        "enable_fusion",
                        "max_input_tokens",
                    ],
                )?;
                if value.get("encoding_format").is_some_and(|v| v != "float") {
                    return Err(RelayError::Unsupported("embedding encoding must be float"));
                }
                let inputs = if value["input"].is_string() {
                    vec![value["input"].clone()]
                } else {
                    value["input"]
                        .as_array()
                        .ok_or(RelayError::Invalid("invalid embedding input"))?
                        .clone()
                };
                let contents = inputs
                    .into_iter()
                    .map(openai_content)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut parameters = json!({});
                for (from, to) in [
                    ("dimensions", "dimension"),
                    ("enable_fusion", "enable_fusion"),
                ] {
                    if let Some(value) = value.get(from) {
                        parameters[to] = value.clone();
                    }
                }
                (contents, parameters)
            }
        };
        if contents.is_empty() || contents.len() > 20 {
            return Err(RelayError::Limit("embedding content count"));
        }
        object(
            &parameters,
            &[
                "dimension",
                "output_type",
                "enable_fusion",
                "fps",
                "instruct",
            ],
        )?;
        if parameters.get("output_type").is_some_and(|v| v != "dense") {
            return Err(RelayError::Unsupported(
                "only dense embeddings are supported",
            ));
        }
        let dimensions = parameters
            .get("dimension")
            .map(|value| {
                value
                    .as_u64()
                    .filter(|v| (1..=4096).contains(v))
                    .map(|v| v as u32)
                    .ok_or(RelayError::Invalid("invalid embedding dimension"))
            })
            .transpose()?;
        let fusion = parameters
            .get("enable_fusion")
            .map(|v| {
                v.as_bool()
                    .ok_or(RelayError::Invalid("invalid embedding fusion"))
            })
            .transpose()?
            .unwrap_or(false);
        if parameters.get("fps").is_some_and(|v| {
            !v.as_f64()
                .is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        }) {
            return Err(RelayError::Invalid("invalid embedding frame rate"));
        }
        let instruction_bytes = parameters
            .get("instruct")
            .map(|v| text(v).map(str::len))
            .transpose()?
            .unwrap_or(0);
        let items = contents.iter().map(item).collect::<Result<Vec<_>, _>>()?;
        if instruction_bytes > 1_048_576
            || items
                .iter()
                .any(|v| matches!(v, Item::Text(bytes) if *bytes > 1_048_576))
        {
            return Err(RelayError::Limit("embedding text bytes"));
        }
        if !fusion {
            parameters.as_object_mut().unwrap().remove("enable_fusion");
        }
        Ok(Self {
            client,
            model: model.to_owned(),
            dimensions,
            input_budget,
            fusion,
            items,
            instruction_bytes,
            wire: json!({"model":model,"input":{"contents":contents},"parameters":parameters}),
        })
    }

    pub(super) fn bind(
        &self,
        contract: &ExternalEmbeddingContract,
        input_limit: u32,
    ) -> Result<Bound, RelayError> {
        let dimensions = self.dimensions.unwrap_or(contract.default_dimensions);
        let images = self
            .items
            .iter()
            .filter(|v| matches!(v, Item::InlineImage(_) | Item::RemoteImage))
            .count();
        let videos = self
            .items
            .iter()
            .filter(|v| matches!(v, Item::RemoteVideo))
            .count();
        let text_bytes = self
            .items
            .iter()
            .filter_map(|v| match v {
                Item::Text(bytes) => Some(*bytes),
                _ => None,
            })
            .sum::<usize>()
            + self.instruction_bytes;
        if self.items.len() > usize::from(contract.max_contents)
            || images > usize::from(contract.max_images)
            || videos > usize::from(contract.max_videos)
            || text_bytes > contract.max_text_bytes as usize
            || !contract.dimensions.contains(&dimensions)
            || (self.fusion && !contract.supports_fusion)
            || self.items.iter().any(|item| match item {
                Item::InlineImage(bytes) => *bytes > contract.max_inline_image_bytes as usize,
                Item::RemoteImage | Item::RemoteVideo => !contract.allow_remote_media,
                Item::Text(_) => false,
            })
        {
            return Err(RelayError::Unsupported(
                "embedding input exceeds the exact supplier contract",
            ));
        }
        // There is no native max_input_tokens control to enforce this smaller
        // number at the supplier. Do not reduce the hold using client intent.
        if self
            .input_budget
            .is_some_and(|budget| budget < u64::from(input_limit))
        {
            return Err(RelayError::Unsupported(
                "embedding budget is below the supplier reservation bound",
            ));
        }
        if input_limit == 0 || u64::from(input_limit) > MAX_EMBEDDING_INPUT_TOKENS {
            return Err(RelayError::RouteUnavailable);
        }
        Ok(Bound {
            provider: contract.provider,
            dimensions,
            count: if self.fusion { 1 } else { self.items.len() },
            input_limit: u64::from(input_limit),
        })
    }

    pub(super) fn outbound(
        &self,
        mapped_model: &str,
        contract: &ExternalEmbeddingContract,
    ) -> Value {
        let mut wire = self.wire.clone();
        wire["model"] = json!(mapped_model);
        // Fixed-dimension models do not support the native dimension field.
        // Requesting that exact default is still representable without it.
        if contract.dimensions.len() == 1 {
            wire["parameters"]
                .as_object_mut()
                .unwrap()
                .remove("dimension");
        }
        wire
    }

    pub(super) fn response(&self, bound: &Bound, bytes: &[u8]) -> Result<Observed, RelayError> {
        let native = unambiguous_json(bytes).map_err(|_| invalid_receipt())?;
        if !native.is_object()
            || native
                .get("code")
                .is_some_and(|v| !matches!(v.as_str(), Some("")))
        {
            return Err(invalid_receipt());
        }
        let request_id = native["request_id"].as_str().ok_or_else(invalid_receipt)?;
        if request_id.is_empty()
            || request_id.len() > 512
            || request_id.chars().any(char::is_control)
        {
            return Err(invalid_receipt());
        }
        let vectors = native["output"]["embeddings"]
            .as_array()
            .ok_or_else(invalid_receipt)?;
        if vectors.len() != bound.count {
            return Err(invalid_receipt());
        }
        let mut indices = BTreeSet::new();
        let mut data = Vec::with_capacity(vectors.len());
        for vector in vectors {
            let index = vector["index"]
                .as_u64()
                .filter(|index| *index < bound.count as u64)
                .ok_or_else(invalid_receipt)?;
            let embedding = vector["embedding"].as_array().ok_or_else(invalid_receipt)?;
            if !indices.insert(index)
                || embedding.len() != bound.dimensions as usize
                || embedding
                    .iter()
                    .any(|n| !n.as_f64().is_some_and(f64::is_finite))
                || vector.get("type").is_some_and(|v| {
                    !matches!(
                        v.as_str(),
                        Some("text" | "image" | "video" | "vl" | "fusion" | "fused")
                    )
                })
            {
                return Err(invalid_receipt());
            }
            data.push(json!({"object":"embedding","index":index,"embedding":embedding}));
        }
        let evidence = receipt_usage(bound.provider, &native["usage"])?;
        if evidence
            .total_input_tokens
            .is_some_and(|total| total > bound.input_limit)
        {
            return Err(invalid_receipt());
        }
        let output = match self.client {
            Client::Native => native,
            Client::OpenAi => {
                let mut usage = json!({});
                if let Some(total) = evidence.total_input_tokens {
                    usage["prompt_tokens"] = json!(total);
                    usage["total_tokens"] = json!(total);
                }
                if evidence.text_tokens.is_some() || evidence.image_tokens.is_some() {
                    let mut details = json!({});
                    if let Some(text) = evidence.text_tokens {
                        details["text_tokens"] = json!(text);
                    }
                    if let Some(image) = evidence.image_tokens {
                        details["image_tokens"] = json!(image);
                    }
                    usage["prompt_tokens_details"] = details;
                }
                json!({"object":"list","model":self.model,"data":data,"usage":usage})
            }
        };
        Ok(Observed {
            output,
            usage: Usage {
                input_tokens: evidence.total_input_tokens,
                output_tokens: Some(0),
                cached_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
                embedding: Some(evidence),
                ..Usage::default()
            },
        })
    }
}

fn receipt_usage(provider: EmbeddingProvider, usage: &Value) -> Result<EmbeddingUsage, RelayError> {
    object(
        usage,
        &[
            "input_tokens",
            "image_tokens",
            "input_tokens_details",
            "output_tokens",
            "total_tokens",
            "image_count",
            "duration",
        ],
    )
    .map_err(|_| invalid_receipt())?;
    if usage
        .get("image_count")
        .is_some_and(|v| v.as_u64().is_none_or(|n| n > 20))
        || usage
            .get("duration")
            .is_some_and(|v| !v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
    {
        return Err(invalid_receipt());
    }
    let input = count(usage, "input_tokens")?;
    let total = count(usage, "total_tokens")?;
    let output = count(usage, "output_tokens")?;
    let (text, image, total_input) = match provider {
        EmbeddingProvider::AliSeparateInputV1 => {
            if usage
                .get("input_tokens_details")
                .is_some_and(|v| !v.is_null())
                || output.is_some_and(|v| v != 0)
            {
                return Err(invalid_receipt());
            }
            let image = count(usage, "image_tokens")?;
            let sum = input
                .zip(image)
                .and_then(|(text, image)| text.checked_add(image));
            if sum.zip(total).is_some_and(|(sum, total)| sum != total) {
                return Err(invalid_receipt());
            }
            (input, image, total.or(sum))
        }
        EmbeddingProvider::AliInclusiveInputV1 => {
            let details = usage.get("input_tokens_details").unwrap_or(&Value::Null);
            if !details.is_null() && !details.is_object() {
                return Err(invalid_receipt());
            }
            if !details.is_null() {
                object(details, &["text_tokens", "image_tokens"]).map_err(|_| invalid_receipt())?;
            }
            let text = count(details, "text_tokens")?;
            let image = count(details, "image_tokens")?;
            if count(usage, "image_tokens")?.is_some_and(|top| image != Some(top)) {
                return Err(invalid_receipt());
            }
            if let (Some(input), Some(total)) = (input, total) {
                if total < input
                    || output.is_some_and(|output| input.checked_add(output) != Some(total))
                {
                    return Err(invalid_receipt());
                }
            }
            (text, image, input)
        }
    };
    let evidence = EmbeddingUsage {
        provider,
        text_tokens: text,
        image_tokens: image,
        total_input_tokens: total_input,
    };
    if !evidence.valid() {
        return Err(invalid_receipt());
    }
    Ok(evidence)
}
fn count(value: &Value, key: &str) -> Result<Option<u64>, RelayError> {
    value
        .get(key)
        .map(|v| {
            v.as_u64()
                .filter(|n| *n <= MAX_EMBEDDING_INPUT_TOKENS)
                .ok_or_else(invalid_receipt)
        })
        .transpose()
}
fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, RelayError> {
    let object = value
        .as_object()
        .ok_or(RelayError::Invalid("expected embedding object"))?;
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(RelayError::Unsupported("unsupported embedding field"));
    }
    Ok(object)
}
fn text(value: &Value) -> Result<&str, RelayError> {
    value
        .as_str()
        .ok_or(RelayError::Invalid("expected embedding string"))
}
fn openai_content(value: Value) -> Result<Value, RelayError> {
    if value.is_string() {
        return Ok(json!({"text":value}));
    }
    match value["type"].as_str() {
        Some("text") => {
            object(&value, &["type", "text"])?;
            Ok(json!({"text":text(&value["text"])?}))
        }
        Some("image_url" | "video_url") => {
            let source = if value["type"] == "image_url" {
                "image_url"
            } else {
                "video_url"
            };
            let target = if source == "image_url" {
                "image"
            } else {
                "video"
            };
            object(&value, &["type", source])?;
            let location = if value[source].is_object() {
                object(&value[source], &["url"])?;
                text(&value[source]["url"])?
            } else {
                text(&value[source])?
            };
            Ok(json!({target:location}))
        }
        _ => Err(RelayError::Unsupported("unsupported embedding input kind")),
    }
}
fn item(value: &Value) -> Result<Item, RelayError> {
    let object = object(value, &["text", "image", "video"])?;
    if object.len() != 1 {
        return Err(RelayError::Unsupported(
            "embedding items must each contain one modality",
        ));
    }
    let (kind, value) = object.iter().next().unwrap();
    let source = text(value)?;
    match kind.as_str() {
        "text" if !source.is_empty() => Ok(Item::Text(source.len())),
        "image" if source.starts_with("data:") => {
            let (header, encoded) = source
                .split_once(',')
                .ok_or(RelayError::Invalid("invalid inline embedding image"))?;
            if !matches!(
                header,
                "data:image/png;base64"
                    | "data:image/jpeg;base64"
                    | "data:image/webp;base64"
                    | "data:image/bmp;base64"
                    | "data:image/tiff;base64"
            ) || encoded.len() > 7 * 1_048_576
            {
                return Err(RelayError::Invalid("unsupported inline embedding image"));
            }
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| RelayError::Invalid("invalid embedding image encoding"))?;
            let signature = match header {
                "data:image/png;base64" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "data:image/jpeg;base64" => bytes.starts_with(&[0xff, 0xd8]),
                "data:image/webp;base64" => {
                    bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
                }
                "data:image/bmp;base64" => bytes.starts_with(b"BM"),
                "data:image/tiff;base64" => {
                    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
                }
                _ => false,
            };
            if !signature || bytes.len() > 5 * 1_048_576 {
                return Err(RelayError::Invalid("invalid embedding image bytes"));
            }
            Ok(Item::InlineImage(bytes.len()))
        }
        "image" | "video" => {
            let url = reqwest::Url::parse(source)
                .map_err(|_| RelayError::Invalid("invalid embedding media URL"))?;
            if source.len() > 8192
                || source.chars().any(char::is_control)
                || !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(RelayError::Invalid("invalid embedding media URL"));
            }
            Ok(if kind == "image" {
                Item::RemoteImage
            } else {
                Item::RemoteVideo
            })
        }
        _ => Err(RelayError::Invalid("invalid embedding input")),
    }
}
fn invalid_receipt() -> RelayError {
    RelayError::Protocol("invalid or contradictory embedding receipt")
}

#[cfg(test)]
#[path = "ali_embedding_tests.rs"]
mod tests;
