//! Frozen native System One contract, not an OpenAI Chat conversion.
//! Source: official TypeSafe OpenAPI ee599512cbea3a198b34af3f60addcd72b381d3521196f93c70ae83185c057c3
//! plus frozen images.md/errors.md under output/remaining-20261020/models/jev-20261020.
use crate::{
    model_http::image_provider::{self, ImageMediaType, ImageUpload},
    relay::{RelayError, Usage},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Map, Value, value::RawValue};
use std::collections::{BTreeMap, BTreeSet};
const BODY: usize = 32 * 1024 * 1024;
const IMAGE: usize = 16 * 1024 * 1024;
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(crate) struct DangerZone {
    pub images: bool,
    pub long_context: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageFormat {
    Png,
    Jpg,
    Jpeg,
}
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))] // Tests inspect part ownership; runtime keeps bytes on `wire`/`uploads`.
pub(crate) enum AssetSource {
    Inline,
    Part(String),
}
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))] // Tests inspect the stored source after format checks.
pub(crate) struct Asset {
    pub source: AssetSource,
}
#[derive(Clone)]
pub(crate) struct Upload {
    pub part: String,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Request {
    wire: Value,
    flags: DangerZone,
    #[cfg_attr(not(test), allow(dead_code))]
    // Tests read assets; runtime serves `uploads` and `wire`.
    assets: BTreeMap<String, Asset>,
    uploads: Vec<Upload>,
}
pub(crate) struct Response {
    pub output: Value,
    pub usage: Usage,
}

// RawValue validates JSON syntax and preserves each value's real token kind.
// Typed maps reject duplicates without interpreting arbitrary_precision's
// private numeric map carrier or reserving any user-controlled object keys.
struct RawObject<'a>(BTreeMap<String, &'a RawValue>);
impl<'de> Deserialize<'de> for RawObject<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RawObject<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object with unique keys")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = a.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                    values.insert(key, a.next_value::<&'de RawValue>()?);
                }
                Ok(RawObject(values))
            }
        }
        d.deserialize_map(V)
    }
}
fn invalid_json() -> RelayError {
    RelayError::Invalid("invalid or ambiguous System One JSON")
}
fn decode_raw(raw: &RawValue, depth: usize) -> Result<Value, RelayError> {
    if depth > 128 {
        return Err(RelayError::Invalid(
            "System One JSON nesting exceeds implementation bound",
        ));
    }
    let raw = raw.get().trim();
    match raw.as_bytes().first() {
        Some(b'{') => {
            let object: RawObject<'_> = serde_json::from_str(raw).map_err(|_| invalid_json())?;
            let mut values = Map::new();
            for (key, value) in object.0 {
                values.insert(key, decode_raw(value, depth + 1)?);
            }
            Ok(Value::Object(values))
        }
        Some(b'[') => {
            let values: Vec<&RawValue> = serde_json::from_str(raw).map_err(|_| invalid_json())?;
            values
                .into_iter()
                .map(|value| decode_raw(value, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Some(b'"') => serde_json::from_str::<String>(raw)
            .map(Value::String)
            .map_err(|_| invalid_json()),
        Some(b't' | b'f') => serde_json::from_str::<bool>(raw)
            .map(Value::Bool)
            .map_err(|_| invalid_json()),
        Some(b'n') => {
            serde_json::from_str::<()>(raw).map_err(|_| invalid_json())?;
            Ok(Value::Null)
        }
        Some(b'-' | b'0'..=b'9') => {
            let number: serde_json::Number =
                serde_json::from_str(raw).map_err(|_| invalid_json())?;
            if !number.as_f64().is_some_and(f64::is_finite) {
                return Err(RelayError::Invalid("nonfinite System One number"));
            }
            Ok(Value::Number(number))
        }
        _ => Err(invalid_json()),
    }
}
fn json(body: &[u8]) -> Result<Value, RelayError> {
    if body.len() > BODY {
        return Err(RelayError::Limit("System One body"));
    }
    if body.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(RelayError::Invalid("System One BOM is unsupported"));
    }
    let raw: &RawValue = serde_json::from_slice(body).map_err(|_| invalid_json())?;
    decode_raw(raw, 0)
}
fn object<'a>(v: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, RelayError> {
    let o = v
        .as_object()
        .ok_or(RelayError::Invalid("expected System One object"))?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(RelayError::Unsupported("unsupported System One field"));
    }
    Ok(o)
}
fn text(v: &Value) -> Result<&str, RelayError> {
    v.as_str()
        .ok_or(RelayError::Invalid("expected System One string"))
}
fn structured(v: &Value, nullable: bool) -> Result<(), RelayError> {
    if !(v.is_string() || v.is_object() || v.is_array() || (nullable && v.is_null())) {
        return Err(RelayError::Invalid("invalid structured System One value"));
    }
    fn walk(v: &Value, depth: usize) -> Result<(), RelayError> {
        match v {
            Value::Object(m) => {
                if depth >= 64 {
                    return Err(RelayError::Invalid("System One nesting exceeds 64"));
                }
                for v in m.values() {
                    walk(v, depth + 1)?;
                }
            }
            Value::Array(a) => {
                if depth >= 64 {
                    return Err(RelayError::Invalid("System One nesting exceeds 64"));
                }
                for v in a {
                    walk(v, depth + 1)?;
                }
            }
            Value::Number(n) if !n.as_f64().is_some_and(f64::is_finite) => {
                return Err(RelayError::Invalid("nonfinite System One number"));
            }
            _ => {}
        }
        Ok(())
    }
    walk(v, 0)
}
fn format(v: &Value) -> Result<ImageFormat, RelayError> {
    match text(v)? {
        "png" => Ok(ImageFormat::Png),
        "jpg" => Ok(ImageFormat::Jpg),
        "jpeg" => Ok(ImageFormat::Jpeg),
        _ => Err(RelayError::Invalid("unsupported System One image format")),
    }
}
fn image_bytes(bytes: &[u8], declared: Option<ImageFormat>) -> Result<(), RelayError> {
    if bytes.len() > IMAGE {
        return Err(RelayError::Limit("System One image bytes"));
    }
    let detected = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        ImageFormat::Jpg
    } else {
        return Err(RelayError::Invalid("System One image must be PNG or JPEG"));
    };
    if declared.is_some_and(|f| (f == ImageFormat::Png) != (detected == ImageFormat::Png)) {
        return Err(RelayError::Invalid(
            "System One image format does not match bytes",
        ));
    }
    let upload = ImageUpload {
        bytes: bytes.to_vec(),
        media_type: if detected == ImageFormat::Png {
            ImageMediaType::Png
        } else {
            ImageMediaType::Jpeg
        },
    };
    let (w, h, _) = image_provider::dimensions(&upload)?;
    if !(280..=1568).contains(&w) || !(280..=1568).contains(&h) {
        return Err(RelayError::Limit("System One image dimensions"));
    }
    if detected == ImageFormat::Png {
        let mut offset = 8usize;
        while offset + 12 <= bytes.len() {
            let n = u32::from_be_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .map_err(|_| RelayError::Invalid("invalid PNG"))?,
            ) as usize;
            let end = offset
                .checked_add(n)
                .and_then(|n| n.checked_add(12))
                .filter(|end| *end <= bytes.len())
                .ok_or(RelayError::Invalid("invalid PNG"))?;
            if &bytes[offset + 4..offset + 8] == b"acTL" {
                return Err(RelayError::Invalid("animated PNG is unsupported"));
            }
            offset = end;
        }
    }
    Ok(())
}
fn references(
    v: &Value,
    assets: &BTreeMap<String, Asset>,
    counts: &mut BTreeMap<String, usize>,
) -> Result<(), RelayError> {
    match v {
        Value::String(s) => {
            if assets.contains_key(s) {
                *counts.entry(s.clone()).or_default() += 1;
                return Ok(());
            }
            let mut cursor = 0;
            while cursor < s.len() {
                let mut matches: Vec<(usize, &String)> = assets
                    .keys()
                    .filter_map(|key| s[cursor..].find(key).map(|at| (cursor + at, key)))
                    .collect();
                matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.len().cmp(&a.1.len())));
                let Some((at, key)) = matches.first() else {
                    break;
                };
                // The published contract does not define a tie breaker for
                // two references starting at the same position. Reject this
                // ambiguity rather than invent an expansion/token count.
                if matches.get(1).is_some_and(|next| next.0 == *at) {
                    return Err(RelayError::Unsupported(
                        "ambiguous overlapping image references",
                    ));
                }
                *counts.entry((*key).clone()).or_default() += 1;
                cursor = *at + key.len();
            }
        }
        Value::Array(a) => {
            for v in a {
                references(v, assets, counts)?;
            }
        }
        Value::Object(m) => {
            for v in m.values() {
                references(v, assets, counts)?;
            }
        }
        _ => {}
    }
    Ok(())
}
impl Request {
    pub(crate) fn parse_json(body: &[u8], flags: DangerZone) -> Result<Self, RelayError> {
        Self::parse(body, flags, Vec::new(), false)
    }
    pub(crate) fn parse_multipart_json(
        body: &[u8],
        flags: DangerZone,
        uploads: Vec<Upload>,
    ) -> Result<Self, RelayError> {
        Self::parse(body, flags, uploads, true)
    }
    fn parse(
        body: &[u8],
        flags: DangerZone,
        uploads: Vec<Upload>,
        multipart: bool,
    ) -> Result<Self, RelayError> {
        let wire = json(body)?;
        object(&wire, &["model", "state", "questions", "assets"])?;
        let model = text(&wire["model"])?;
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(RelayError::Invalid("invalid System One model"));
        }
        structured(&wire["state"], false)?;
        let questions = wire["questions"]
            .as_object()
            .filter(|q| !q.is_empty())
            .ok_or(RelayError::Invalid("System One questions required"))?;
        let mut box_question = false;
        for q in questions.values() {
            let kind = text(&q["type"])?;
            object(
                q,
                if kind == "box" {
                    &["type", "instructions"]
                } else {
                    &["type", "instructions", "criteria"]
                },
            )?;
            if let Some(i) = q.get("instructions") {
                structured(i, true)?;
            }
            match kind {
                "noul" => {
                    if let Some(c) = q.get("criteria").filter(|v| !v.is_null()) {
                        object(c, &["true", "false"])?;
                        for v in c
                            .as_object()
                            .ok_or(RelayError::Invalid("invalid Noul criteria"))?
                            .values()
                        {
                            structured(v, true)?;
                        }
                    }
                }
                "choice" => {
                    let c = q["criteria"]
                        .as_object()
                        .filter(|c| !c.is_empty() && c.len() <= 255)
                        .ok_or(RelayError::Invalid("Choice needs 1..255 criteria"))?;
                    for v in c.values() {
                        structured(v, true)?;
                    }
                }
                "score" => {
                    let c = q["criteria"]
                        .as_array()
                        .filter(|a| !a.is_empty() && a.len() <= 10)
                        .ok_or(RelayError::Invalid("Score needs 1..10 criteria"))?;
                    for v in c {
                        structured(v, false)?;
                    }
                }
                "box" => box_question = true,
                _ => {
                    return Err(RelayError::Unsupported(
                        "unsupported System One question type",
                    ));
                }
            }
        }
        if (wire.get("assets").is_some() || box_question) && !flags.images {
            return Err(RelayError::Unsupported(
                "System One images require explicit danger-zone flag",
            ));
        }
        let mut parts = BTreeMap::new();
        let mut upload_total = body.len();
        for upload in &uploads {
            if upload.part.is_empty()
                || upload.part == "request"
                || parts.insert(upload.part.clone(), upload).is_some()
            {
                return Err(RelayError::Invalid("ambiguous System One upload part"));
            }
            if !matches!(
                upload.content_type.as_deref(),
                None | Some("image/png" | "image/jpeg" | "image/jpg" | "application/octet-stream")
            ) {
                return Err(RelayError::Invalid(
                    "invalid System One image part content type",
                ));
            }
            upload_total = upload_total
                .checked_add(upload.bytes.len())
                .filter(|n| *n <= BODY)
                .ok_or(RelayError::Limit("System One uploads"))?;
        }
        let mut assets = BTreeMap::new();
        let mut ref_bytes = 0usize;
        let mut used_parts = BTreeSet::new();
        let mut image_total = 0usize;
        if let Some(raw) = wire.get("assets") {
            let raw = raw
                .as_object()
                .filter(|a| !a.is_empty())
                .ok_or(RelayError::Invalid("invalid System One assets"))?;
            for (reference, a) in raw {
                if reference.trim().is_empty() || reference.len() > 1024 {
                    return Err(RelayError::Limit("System One asset reference"));
                }
                ref_bytes = ref_bytes
                    .checked_add(reference.len())
                    .filter(|n| *n <= 65536)
                    .ok_or(RelayError::Limit("System One reference lengths"))?;
                object(a, &["type", "format", "data_base64", "part"])?;
                if a.get("type").is_some_and(|t| t != "image") {
                    return Err(RelayError::Invalid("invalid System One asset type"));
                }
                let declared = a.get("format").map(format).transpose()?;
                let (source, size) = if let Some(part) = a.get("part") {
                    let part = text(part)?;
                    if !multipart || part.is_empty() || !used_parts.insert(part.to_owned()) {
                        return Err(RelayError::Invalid(
                            "invalid System One asset part ownership",
                        ));
                    }
                    let upload = parts
                        .get(part)
                        .ok_or(RelayError::Invalid("missing System One upload"))?;
                    image_bytes(&upload.bytes, declared)?;
                    (AssetSource::Part(part.to_owned()), upload.bytes.len())
                } else {
                    if declared.is_none() {
                        return Err(RelayError::Invalid("inline image format required"));
                    }
                    let data = text(&a["data_base64"])?;
                    let bytes = STANDARD
                        .decode(data)
                        .map_err(|_| RelayError::Invalid("invalid standard image base64"))?;
                    if STANDARD.encode(&bytes) != data {
                        return Err(RelayError::Invalid("noncanonical image base64"));
                    }
                    image_bytes(&bytes, declared)?;
                    let size = bytes.len();
                    (AssetSource::Inline, size)
                };
                image_total = image_total
                    .checked_add(size)
                    .filter(|n| *n <= BODY)
                    .ok_or(RelayError::Limit("System One all image bytes"))?;
                assets.insert(reference.clone(), Asset { source });
            }
        }
        if box_question && assets.is_empty() {
            return Err(RelayError::Invalid(
                "System One Box questions require an image",
            ));
        }
        if used_parts.len() != uploads.len() {
            return Err(RelayError::Invalid("unused System One upload"));
        }
        let mut counts = BTreeMap::new();
        references(&wire["state"], &assets, &mut counts)?;
        for q in questions.values() {
            for k in ["instructions", "criteria"] {
                if let Some(v) = q.get(k) {
                    references(v, &assets, &mut counts)?;
                }
            }
        }
        if assets.keys().any(|key| !counts.contains_key(key)) {
            return Err(RelayError::Invalid("unreferenced System One asset"));
        }
        // This is only the native structured-text serialization budget.
        // Image processing can shrink OR enlarge a file. The supplier's actual
        // processed/expanded image limit cannot be inferred from upload bytes
        // times reference count, so do not reject requests using that estimate.
        serde_json::to_vec(&wire["state"])
            .map_err(|_| RelayError::Invalid("invalid state"))?
            .len()
            .checked_add(
                serde_json::to_vec(&wire["questions"])
                    .map_err(|_| RelayError::Invalid("invalid questions"))?
                    .len(),
            )
            .filter(|n| *n <= BODY)
            .ok_or(RelayError::Limit("System One expanded request"))?;
        Ok(Self {
            wire,
            flags,
            assets,
            uploads,
        })
    }
    pub(crate) fn model(&self) -> &str {
        self.wire["model"].as_str().expect("validated model")
    }
    pub(crate) fn wire(&self) -> &Value {
        &self.wire
    }
    #[cfg_attr(not(test), allow(dead_code))] // Tests read assets; runtime serves `uploads` and `wire`.
    pub(crate) fn assets(&self) -> &BTreeMap<String, Asset> {
        &self.assets
    }
    pub(crate) fn uploads(&self) -> &[Upload] {
        &self.uploads
    }
    pub(crate) fn flags(&self) -> DangerZone {
        self.flags
    }
}
fn probability(v: &Value) -> Result<f64, RelayError> {
    v.as_f64()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
        .ok_or(RelayError::Protocol("invalid System One probability"))
}
fn same_keys(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a.len() == b.len() && a.keys().all(|key| b.contains_key(key))
}
impl Response {
    pub(crate) fn parse(request: &Request, body: &[u8]) -> Result<Self, RelayError> {
        fn validate(request: &Request, body: &[u8]) -> Result<Response, RelayError> {
            let output = json(body)?;
            object(&output, &["model", "answers", "usage"])?;
            let model = text(&output["model"])?;
            if model.is_empty() || model.len() > 256 {
                return Err(RelayError::Protocol("invalid System One response model"));
            }
            let answers = output["answers"]
                .as_object()
                .ok_or(RelayError::Protocol("System One answers required"))?;
            let questions = request.wire["questions"]
                .as_object()
                .ok_or(RelayError::Protocol("invalid original questions"))?;
            if !same_keys(answers, questions) {
                return Err(RelayError::Protocol("System One answer set mismatch"));
            }
            for (id, answer) in answers {
                let question = &questions[id];
                let kind = text(&answer["type"])?;
                if answer["type"] != question["type"] {
                    return Err(RelayError::Protocol("System One answer type mismatch"));
                }
                match kind {
                    "noul" => {
                        object(answer, &["type", "noul"])?;
                        probability(&answer["noul"])?;
                    }
                    "choice" => {
                        object(answer, &["type", "choice", "confidence", "probabilities"])?;
                        probability(&answer["confidence"])?;
                        let choices = question["criteria"]
                            .as_object()
                            .ok_or(RelayError::Protocol("invalid original choices"))?;
                        if !choices.contains_key(text(&answer["choice"])?) {
                            return Err(RelayError::Protocol("System One choice outside criteria"));
                        }
                        let p = answer["probabilities"]
                            .as_object()
                            .ok_or(RelayError::Protocol("missing Choice probabilities"))?;
                        if !same_keys(p, choices) {
                            return Err(RelayError::Protocol("Choice probability set mismatch"));
                        }
                        let chosen = text(&answer["choice"])?;
                        let selected = probability(&p[chosen])?;
                        for v in p.values() {
                            if probability(v)? > selected {
                                return Err(RelayError::Protocol(
                                    "Choice answer is not a highest-probability option",
                                ));
                            }
                        }
                    }
                    "score" => {
                        object(
                            answer,
                            &["type", "score", "confidence", "legend", "probabilities"],
                        )?;
                        probability(&answer["confidence"])?;
                        let criteria = question["criteria"]
                            .as_array()
                            .ok_or(RelayError::Protocol("invalid original score levels"))?;
                        let score = answer["score"]
                            .as_f64()
                            .filter(|n| {
                                n.is_finite() && *n >= 0.0 && *n <= (criteria.len() - 1) as f64
                            })
                            .ok_or(RelayError::Protocol("score outside original levels"))?;
                        let _ = score;
                        let legend = answer["legend"]
                            .as_object()
                            .ok_or(RelayError::Protocol("missing Score legend"))?;
                        let p = answer["probabilities"]
                            .as_object()
                            .ok_or(RelayError::Protocol("missing Score probabilities"))?;
                        if legend.len() != criteria.len() || !same_keys(legend, p) {
                            return Err(RelayError::Protocol(
                                "Score legend/probability set mismatch",
                            ));
                        }
                        for (index, criterion) in criteria.iter().enumerate() {
                            let key = index.to_string();
                            if legend.get(&key) != Some(criterion) {
                                return Err(RelayError::Protocol(
                                    "Score legend changed original criteria",
                                ));
                            }
                            probability(&p[&key])?;
                        }
                    }
                    "box" => {
                        object(answer, &["type", "box", "confidence"])?;
                        if !answer.get("confidence").is_some_and(Value::is_null) {
                            probability(&answer["confidence"])?;
                        }
                        let b = &answer["box"];
                        object(b, &["top", "bottom", "left", "right"])?;
                        let top = probability(&b["top"])?;
                        let bottom = probability(&b["bottom"])?;
                        let left = probability(&b["left"])?;
                        let right = probability(&b["right"])?;
                        if top >= bottom || left >= right {
                            return Err(RelayError::Protocol("inverted System One box"));
                        }
                    }
                    _ => return Err(RelayError::Protocol("unknown System One answer type")),
                }
            }
            object(&output["usage"], &["input_tokens", "output_tokens"])?;
            let input = output["usage"]["input_tokens"]
                .as_u64()
                .ok_or(RelayError::Protocol(
                    "System One input usage missing or invalid",
                ))?;
            let out = output["usage"]["output_tokens"]
                .as_u64()
                .ok_or(RelayError::Protocol(
                    "System One output usage missing or invalid",
                ))?;
            let usage = Usage {
                input_tokens: Some(input),
                output_tokens: Some(out),
                ..Usage::default()
            };
            usage.validate()?;
            Ok(Response { output, usage })
        }
        validate(request, body).map_err(|error| match error {
            RelayError::Limit(_) => RelayError::Limit("System One response"),
            _ => RelayError::Protocol("invalid native System One response"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> Request {
        let value = json!({"model":"jev-1.14.0","state":{"nested":["message",true,3],"image":"box-image-reference"},"assets":{"box-image-reference":{"format":"png","data_base64":STANDARD.encode(png())}},"questions":{"n":{"type":"noul"},"c":{"type":"choice","criteria":{"x":null,"y":{"description":"yes"}}},"s":{"type":"score","criteria":["low",{"description":"high"}]},"b":{"type":"box"}}});
        Request::parse_json(
            &serde_json::to_vec(&value).unwrap(),
            DangerZone {
                images: true,
                long_context: false,
            },
        )
        .unwrap()
    }
    fn response() -> Value {
        json!({"model":"jev-1.14","answers":{"n":{"type":"noul","noul":0.3},"c":{"type":"choice","choice":"y","confidence":0.9,"probabilities":{"x":0.1,"y":0.9}},"s":{"type":"score","score":0.8,"confidence":0.9,"legend":{"0":"low","1":{"description":"high"}},"probabilities":{"0":0.2,"1":0.8}},"b":{"type":"box","box":{"top":0.1,"bottom":0.9,"left":0.2,"right":0.7},"confidence":null}},"usage":{"input_tokens":50,"output_tokens":12}})
    }
    fn png() -> Vec<u8> {
        STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAARgAAAEYCAYAAACHjumMAAACqklEQVR42u3UMQ0AAAzDsPIn3YKY9tkQciQFeBIJAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGMBgAAwGMBjAYAAMBjAYwGAADAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGMBgAgwEMBjAYAIMBDAYwGACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQwGwGAAgwEMBsBgAIMBDAbAYACDAQxGAsBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBMBjAYACDATAYwGAAgwEwGMBgAIMBuBlknX59IehThQAAAABJRU5ErkJggg==").unwrap()
    }
    #[test]
    fn systemone_native_png_inline_and_multipart_preserve_references_and_part_priority() {
        let flags = DangerZone {
            images: true,
            long_context: false,
        };
        let bytes = png();
        let body = json!({"model":"jev-latest","state":{"camera":"unique-image-reference"},"assets":{"unique-image-reference":{"format":"png","data_base64":STANDARD.encode(&bytes)}},"questions":{"q":{"type":"noul","instructions":{"compare":["unique-image-reference"]}}}});
        let parsed = Request::parse_json(&serde_json::to_vec(&body).unwrap(), flags).unwrap();
        assert_eq!(parsed.assets().len(), 1);
        assert_eq!(parsed.wire(), &body);
        let mut body = body;
        body["assets"]["unique-image-reference"] =
            json!({"part":"camera","data_base64":"deliberately-ignored-invalid-base64"});
        let parsed = Request::parse_multipart_json(
            &serde_json::to_vec(&body).unwrap(),
            flags,
            vec![Upload {
                part: "camera".into(),
                bytes: bytes.clone(),
                content_type: Some("application/octet-stream".into()),
            }],
        )
        .unwrap();
        assert_eq!(parsed.uploads()[0].bytes, bytes);
        assert!(
            matches!(&parsed.assets()["unique-image-reference"].source,AssetSource::Part(p) if p=="camera")
        );
        body["state"] = json!("no matching reference");
        body["questions"]["q"]
            .as_object_mut()
            .unwrap()
            .remove("instructions");
        assert!(
            Request::parse_multipart_json(
                &serde_json::to_vec(&body).unwrap(),
                flags,
                vec![Upload {
                    part: "camera".into(),
                    bytes: png(),
                    content_type: None
                }]
            )
            .is_err()
        );
    }
    #[test]
    fn systemone_native_arbitrary_precision_numbers_and_user_carrier_keys_remain_distinct() {
        let body=br#"{"model":"m","state":{"probability":0.3,"precise":18446744073709551615,"literal":{"$serde_json::private::Number":"not-a-number"}},"questions":{"q":{"type":"noul"}}}"#;
        let request = Request::parse_json(body, DangerZone::default()).unwrap();
        assert!(request.wire()["state"]["probability"].is_number());
        assert_eq!(request.wire()["state"]["probability"].as_f64(), Some(0.3));
        assert_eq!(request.wire()["state"]["precise"].as_u64(), Some(u64::MAX));
        assert!(request.wire()["state"]["literal"].is_object());
        assert_eq!(
            request.wire()["state"]["literal"]["$serde_json::private::Number"],
            "not-a-number"
        );
        assert!(json(br#"{"ignored":{"number":1e400}}"#).is_err());
    }
    #[test]
    fn systemone_native_repeated_references_are_not_rejected_by_raw_weighted_image_guess() {
        let image = png();
        let uses = BODY / image.len() + 1;
        assert!(
            image.len() * uses > BODY,
            "the old raw-weighted estimate must exceed32MiB"
        );
        let reference = "legal-image-reference";
        let state = Value::Array((0..uses).map(|_| json!(reference)).collect());
        let wire = json!({"model":"jev-1.14.0","state":state,"questions":{"q":{"type":"noul"}},"assets":{reference:{"format":"png","data_base64":STANDARD.encode(&image)}}});
        let body = serde_json::to_vec(&wire).unwrap();
        assert!(body.len() < BODY);
        let request = Request::parse_json(
            &body,
            DangerZone {
                images: true,
                long_context: false,
            },
        )
        .unwrap();
        let mut counts = BTreeMap::new();
        references(&request.wire()["state"], request.assets(), &mut counts).unwrap();
        assert_eq!(counts[reference], uses);
        assert_eq!(request.wire(), &wire);
        // This establishes local admission, not supplier processed-size acceptance.
    }
    #[test]
    fn systemone_native_structured_questions_are_preserved() {
        let r = request();
        assert!(r.wire()["state"].is_object());
        assert_eq!(r.wire()["questions"]["c"]["criteria"]["x"], Value::Null);
        assert!(r.flags().images);
        let output = response();
        let parsed = Response::parse(&r, &serde_json::to_vec(&output).unwrap()).unwrap();
        assert_eq!(parsed.output, output);
        assert_eq!(parsed.usage.cached_input_tokens, None);
        assert_eq!(parsed.usage.input_tokens, Some(50));
    }
    #[test]
    fn systemone_native_rejects_duplicate_bom_nonfinite_depth_and_unknown_controls() {
        for b in [
            r#"{"model":"m","state":{"x":1,"x":2},"questions":{"q":{"type":"noul"}}}"#,
            r#"{"model":"m","state":[1e400],"questions":{"q":{"type":"noul"}}}"#,
            r#"{"model":"m","state":"s","questions":{"q":{"type":"noul"}},"payer":1}"#,
        ] {
            assert!(
                Request::parse_json(b.as_bytes(), DangerZone::default()).is_err(),
                "invalid request unexpectedly admitted: {b}"
            );
        }
        let mut body = vec![0xef, 0xbb, 0xbf];
        body.extend_from_slice(br#"{"model":"m","state":"s","questions":{"q":{"type":"noul"}}}"#);
        assert!(Request::parse_json(&body, DangerZone::default()).is_err());
        let mut state = json!("x");
        for _ in 0..65 {
            state = json!([state]);
        }
        assert!(
            Request::parse_json(
                &serde_json::to_vec(
                    &json!({"model":"m","state":state,"questions":{"q":{"type":"noul"}}})
                )
                .unwrap(),
                DangerZone::default()
            )
            .is_err()
        );
    }
    #[test]
    fn systemone_native_box_requires_explicit_images_flag() {
        let request = request();
        let body = serde_json::to_vec(request.wire()).unwrap();
        assert!(Request::parse_json(&body, DangerZone::default()).is_err());
        assert!(
            Request::parse_json(
                &body,
                DangerZone {
                    images: true,
                    long_context: true
                }
            )
            .is_ok()
        );
    }
    #[test]
    fn systemone_native_box_rejects_missing_image_and_supports_multiple_images() {
        let flags = DangerZone {
            images: true,
            long_context: false,
        };
        let no_image =
            br#"{"model":"jev-1.14.0","state":"find the mug","questions":{"q":{"type":"box"}}}"#;
        assert!(matches!(
            Request::parse_json(no_image, flags),
            Err(RelayError::Invalid(
                "System One Box questions require an image"
            ))
        ));
        let mut wire = request().wire().clone();
        wire["assets"]["second-image-reference"] =
            json!({"format":"png","data_base64":STANDARD.encode(png())});
        wire["state"]["second"] = json!("second-image-reference");
        wire["questions"]["b"]["instructions"] =
            json!("Find the mug in `image`, compare it with `second`.");
        assert_eq!(
            Request::parse_json(&serde_json::to_vec(&wire).unwrap(), flags)
                .unwrap()
                .assets()
                .len(),
            2
        );
    }
    #[test]
    fn systemone_native_box_rejects_zero_area_edges() {
        let request = request();
        for (edge, value) in [("bottom", 0.1), ("right", 0.2)] {
            let mut answer = response();
            answer["answers"]["b"]["box"][edge] = json!(value);
            assert!(Response::parse(&request, &serde_json::to_vec(&answer).unwrap()).is_err());
        }
    }
    #[test]
    fn systemone_native_scientific_numbers_preserve_precision_without_spurious_expansion() {
        let mut body = br#"{"model":"jev-1.14.0","state":{"pad":""#.to_vec();
        body.resize(BODY - 2048, b'x');
        body.extend_from_slice(br#"","values":["#);
        for index in 0..256 {
            if index > 0 {
                body.push(b',');
            }
            body.extend_from_slice(b"1e15");
        }
        body.extend_from_slice(br#"]},"questions":{"q":{"type":"noul"}}}"#);
        assert!(body.len() < BODY);
        let request = Request::parse_json(&body, DangerZone::default()).unwrap();
        assert!(request.wire()["state"]["values"][0].is_number());
        assert_eq!(request.wire()["state"]["values"][0].as_f64(), Some(1e15));
        let expanded = serde_json::to_vec(&request.wire()["state"]).unwrap().len()
            + serde_json::to_vec(&request.wire()["questions"])
                .unwrap()
                .len();
        assert!(expanded <= BODY);
    }
    #[test]
    fn systemone_native_request_body_cap_is_inclusive_and_not_bypassed_by_whitespace() {
        let mut body = br#"{"model":"m","state":""#.to_vec();
        let end = br#"","questions":{"q":{"type":"noul"}}}"#;
        body.resize(BODY - end.len(), b'x');
        body.extend_from_slice(end);
        assert_eq!(body.len(), BODY);
        assert!(Request::parse_json(&body, DangerZone::default()).is_ok());
        body.push(b' ');
        assert!(matches!(
            Request::parse_json(&body, DangerZone::default()),
            Err(RelayError::Limit("System One body"))
        ));
    }
    #[test]
    fn systemone_native_response_question_types_probabilities_and_usage_are_complete() {
        let r = request();
        for key in ["n", "c", "s", "b"] {
            let mut v = response();
            v["answers"].as_object_mut().unwrap().remove(key);
            assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = response();
        v["answers"]["n"]["noul"] = json!(1.1);
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = response();
        v["usage"].as_object_mut().unwrap().remove("input_tokens");
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = response();
        v["usage"]["input_tokens"] = json!(-1);
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = response();
        v["answers"]["c"]["type"] = json!("noul");
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn systemone_native_score_legend_and_box_coordinates_cannot_change() {
        let r = request();
        let mut v = response();
        v["answers"]["s"]["legend"]["0"] = json!("changed");
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = response();
        v["answers"]["b"]["box"]["left"] = json!(0.8);
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = response();
        v["answers"]["c"]["probabilities"]
            .as_object_mut()
            .unwrap()
            .remove("x");
        assert!(Response::parse(&r, &serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn systemone_native_assets_require_reference_and_exclusive_part_ownership() {
        let body=br#"{"model":"m","state":"s","questions":{"q":{"type":"noul"}},"assets":{"ref":{"part":"photo"}}}"#;
        assert!(
            Request::parse_json(
                body,
                DangerZone {
                    images: true,
                    long_context: false
                }
            )
            .is_err()
        );
        assert!(
            Request::parse_multipart_json(
                body,
                DangerZone {
                    images: true,
                    long_context: false
                },
                vec![]
            )
            .is_err()
        );
        let unused = Upload {
            part: "photo".into(),
            bytes: vec![],
            content_type: None,
        };
        assert!(
            Request::parse_multipart_json(
                br#"{"model":"m","state":"s","questions":{"q":{"type":"noul"}}}"#,
                DangerZone::default(),
                vec![unused]
            )
            .is_err()
        );
    }
}
