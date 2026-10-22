use super::*;

fn contract(provider: EmbeddingProvider) -> ExternalEmbeddingContract {
    ExternalEmbeddingContract {
        provider,
        contract_revision: "synthetic-embedding-v1".into(),
        max_contents: 20,
        max_images: 5,
        max_videos: 1,
        max_text_bytes: 32_000,
        max_inline_image_bytes: 5 * 1_048_576,
        default_dimensions: 4,
        dimensions: vec![2, 4],
        supports_fusion: true,
        allow_remote_media: true,
    }
}
fn parse(client: Client, value: Value) -> Request {
    Request::parse(client, &serde_json::to_vec(&value).unwrap()).unwrap()
}
fn native(usage: Value) -> Value {
    json!({"request_id":"synthetic-embedding","output":{"embeddings":[{"index":0,"embedding":[0.123456789,-0.5,1.0,0.0],"type":"vl"}]},"usage":usage})
}
fn rendered(
    request: &Request,
    contract: &ExternalEmbeddingContract,
    value: Value,
) -> Result<Observed, RelayError> {
    let bound = request.bind(contract, 2000).unwrap();
    request.response(&bound, &serde_json::to_vec(&value).unwrap())
}

#[test]
fn openai_inputs_become_native_modalities_and_core_budget_is_not_forwarded() {
    let request = parse(
        Client::OpenAi,
        json!({
            "model":"public-model","input":["text",{"type":"image_url","image_url":{"url":"https://example.test/image.png"}},{"type":"video_url","video_url":"https://example.test/video.mp4"}],
            "dimensions":2,"encoding_format":"float","max_input_tokens":2000,"enable_fusion":true
        }),
    );
    let contract = contract(EmbeddingProvider::AliSeparateInputV1);
    let bound = request.bind(&contract, 2000).unwrap();
    assert_eq!(
        (bound.count, bound.dimensions, bound.input_limit),
        (1, 2, 2000)
    );
    let wire = request.outbound("actual-native-model", &contract);
    assert_eq!(wire["model"], "actual-native-model");
    assert_eq!(
        wire["input"]["contents"],
        json!([{"text":"text"},{"image":"https://example.test/image.png"},{"video":"https://example.test/video.mp4"}])
    );
    assert_eq!(
        wire["parameters"],
        json!({"dimension":2,"enable_fusion":true})
    );
    assert!(wire.get("max_input_tokens").is_none());
    let small = parse(
        Client::OpenAi,
        json!({"model":"alias","input":"text","max_input_tokens":1999}),
    );
    assert!(small.bind(&contract, 2000).is_err());
}

#[test]
fn exact_supplier_content_media_and_dimension_bounds_are_enforced_before_dispatch() {
    let request = parse(
        Client::Native,
        json!({"model":"alias","input":{"contents":[{"text":"abcd"},{"image":"https://example.test/image"}]},"parameters":{"dimension":2}}),
    );
    let mut limits = contract(EmbeddingProvider::AliSeparateInputV1);
    assert!(request.bind(&limits, 2000).is_ok());
    limits.max_contents = 1;
    assert!(request.bind(&limits, 2000).is_err());
    limits.max_contents = 20;
    limits.max_text_bytes = 3;
    assert!(request.bind(&limits, 2000).is_err());
    limits.max_text_bytes = 100;
    limits.max_images = 0;
    assert!(request.bind(&limits, 2000).is_err());
    limits.max_images = 5;
    limits.allow_remote_media = false;
    assert!(request.bind(&limits, 2000).is_err());
    limits.allow_remote_media = true;
    limits.dimensions = vec![4];
    assert!(request.bind(&limits, 2000).is_err());
    let fixed = parse(
        Client::OpenAi,
        json!({"model":"alias","input":"x","dimensions":4}),
    );
    assert!(fixed.bind(&limits, 2000).is_ok());
    assert!(
        fixed.outbound("provider", &limits)["parameters"]
            .get("dimension")
            .is_none()
    );
}

#[test]
fn inline_images_are_decoded_and_bounded_and_unrepresentable_inputs_reject() {
    let png = STANDARD.encode(b"\x89PNG\r\n\x1a\nsynthetic-bounded-fixture");
    let request = parse(
        Client::Native,
        json!({"model":"alias","input":{"contents":[{"image":format!("data:image/png;base64,{png}")}]}}),
    );
    let mut limits = contract(EmbeddingProvider::AliSeparateInputV1);
    assert!(request.bind(&limits, 2000).is_ok());
    limits.max_inline_image_bytes = 1;
    assert!(request.bind(&limits, 2000).is_err());
    for value in [
        json!({"model":"alias","input":[1,2,3]}),
        json!({"model":"alias","input":"x","encoding_format":"base64"}),
        json!({"model":"alias","input":[{"type":"image_url","image_url":{"url":"file:///tmp/image.png"}}]}),
        json!({"model":"alias","input":[{"type":"image_url","image_url":{"url":"https://user:secret@example.test/image"}}]}),
        json!({"model":"alias","input":[{"type":"image_url","image_url":{"url":"data:image/png;base64,bm90LWltYWdl"}}]}),
    ] {
        assert!(Request::parse(Client::OpenAi, &serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(
        Request::parse(
            Client::OpenAi,
            br#"{"model":"alias","input":"x","dimensions":2,"dimensions":4}"#
        )
        .is_err()
    );
}

#[test]
fn separate_text_and_media_receipt_is_inclusive_once_and_unknown_is_not_zero() {
    let request = parse(
        Client::OpenAi,
        json!({"model":"public-alias","input":"text"}),
    );
    let contract = contract(EmbeddingProvider::AliSeparateInputV1);
    let observed = rendered(
        &request,
        &contract,
        native(json!({"input_tokens":43,"image_tokens":1247,"total_tokens":1290})),
    )
    .unwrap();
    assert_eq!(observed.usage.input_tokens, Some(1290));
    assert_eq!(observed.usage.embedding.unwrap().text_tokens, Some(43));
    assert_eq!(observed.output["usage"]["prompt_tokens"], 1290);
    assert_eq!(
        observed.output["data"][0]["embedding"][0],
        json!(0.123456789)
    );
    assert_eq!(observed.output["model"], "public-alias");
    assert!(
        observed
            .output
            .to_string()
            .find("ali_separate_input_v1")
            .is_none()
    );
    let unknown = rendered(&request, &contract, native(json!({"input_tokens":43}))).unwrap();
    assert_eq!(unknown.usage.embedding.unwrap().image_tokens, None);
    assert_eq!(unknown.usage.input_tokens, None);
    assert!(!unknown.usage.embedding.unwrap().complete());
    let zero = rendered(
        &request,
        &contract,
        native(json!({"input_tokens":0,"image_tokens":0,"total_tokens":0})),
    )
    .unwrap();
    assert!(zero.usage.embedding.unwrap().complete());
    for usage in [
        json!({"input_tokens":43,"image_tokens":1247,"total_tokens":43}),
        json!({"input_tokens":-1,"image_tokens":0}),
        json!({"input_tokens":0,"image_tokens":1.5}),
        json!({"input_tokens":0,"image_tokens":"1"}),
        json!({"input_tokens":43,"image_tokens":1247,"input_tokens_details":{"text_tokens":43,"image_tokens":1247}}),
    ] {
        assert!(rendered(&request, &contract, native(usage)).is_err());
    }
}

#[test]
fn inclusive_receipt_checks_the_partition_without_charging_native_output_again() {
    let request = parse(
        Client::Native,
        json!({"model":"alias","input":{"contents":[{"text":"text"}]}}),
    );
    let contract = contract(EmbeddingProvider::AliInclusiveInputV1);
    let value = native(
        json!({"input_tokens":903,"input_tokens_details":{"text_tokens":7,"image_tokens":896},"output_tokens":3,"total_tokens":906}),
    );
    let observed = rendered(&request, &contract, value.clone()).unwrap();
    assert_eq!(observed.output, value);
    assert_eq!(observed.usage.input_tokens, Some(903));
    assert_eq!(observed.usage.output_tokens, Some(0));
    assert_eq!(observed.usage.embedding.unwrap().image_tokens, Some(896));
    for usage in [
        json!({"input_tokens":10,"input_tokens_details":{"text_tokens":7,"image_tokens":896},"output_tokens":3,"total_tokens":906}),
        json!({"input_tokens":903,"input_tokens_details":{"text_tokens":7,"image_tokens":896},"output_tokens":3,"total_tokens":903}),
        json!({"input_tokens":903,"input_tokens_details":{"text_tokens":7,"image_tokens":896,"audio_tokens":1}}),
    ] {
        assert!(rendered(&request, &contract, native(usage)).is_err());
    }
}

#[test]
fn malformed_vectors_duplicate_indices_and_native_error_envelopes_never_settle() {
    let request = parse(Client::OpenAi, json!({"model":"alias","input":"text"}));
    let contract = contract(EmbeddingProvider::AliSeparateInputV1);
    let baseline = native(json!({"input_tokens":1,"image_tokens":0}));
    for embedding in [json!([1, 2]), json!([1, 2, 3, "4"]), json!([1, 2, 3, null])] {
        let mut value = baseline.clone();
        value["output"]["embeddings"][0]["embedding"] = embedding;
        assert!(rendered(&request, &contract, value).is_err());
    }
    let mut wrong_index = baseline.clone();
    wrong_index["output"]["embeddings"][0]["index"] = json!(1);
    assert!(rendered(&request, &contract, wrong_index).is_err());
    let mut error = baseline;
    error["code"] = json!("InvalidApiKey");
    assert!(rendered(&request, &contract, error).is_err());
    let request = parse(Client::OpenAi, json!({"model":"alias","input":["a","b"]}));
    let mut duplicate = native(json!({"input_tokens":1,"image_tokens":0}));
    let vector = duplicate["output"]["embeddings"][0].clone();
    duplicate["output"]["embeddings"]
        .as_array_mut()
        .unwrap()
        .push(vector);
    assert!(rendered(&request, &contract, duplicate).is_err());
}
