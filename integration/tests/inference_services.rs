//! Inference endpoints on services, without calling a provider: the test
//! runs a mock of the OpenAI API, which the fixture reaches at
//! `host.docker.internal` (`OXSEARCH_TEST_HOST` changes it), so
//! Elasticsearch's own calls go to it.
//!
//! OpenAI's endpoints are made, used and changed: text embeddings, a
//! completion and its stream, a chat, embeddings, and an update. The other
//! providers that take a URL are put against the same mock, and either made
//! or refused when the mock's answers aren't theirs; Elasticsearch reads
//! each request either way. Elasticsearch's own service is put on the ELSER
//! deployment the default endpoint uses, on a trial. And the region policy.

use std::sync::{Arc, Mutex};

use oxsearch::Error;
use oxsearch::inference::{
    AlibabaCloudServiceSettings, AlibabaCloudServiceType, AlibabaCloudTaskType,
    CustomRequestParams, CustomResponseParams, CustomServiceSettings, CustomServiceType,
    CustomTaskType, DeepSeekServiceSettings, DeepSeekServiceType, ElasticsearchServiceSettings,
    ElasticsearchServiceType, ElasticsearchTaskType, ElserServiceSettings, ElserServiceType,
    ElserTaskType, FireworksAIServiceSettings, FireworksAIServiceType, FireworksAITaskType,
    HuggingFaceServiceSettings, HuggingFaceServiceType, HuggingFaceTaskType, LlamaServiceSettings,
    LlamaServiceType, LlamaTaskType, Message, NvidiaServiceSettings, NvidiaServiceType,
    NvidiaTaskType, OpenAIServiceSettings, OpenAIServiceType, OpenAITaskType,
    OpenShiftAiServiceSettings, OpenShiftAiServiceType, OpenShiftAiTaskType, RegionPolicy,
    RequestChatCompletion, RequestEmbedding, TaskType, TaskTypeDeepSeek, chat_completion_unified,
    completion, delete, delete_region_policy, embedding, get, get_region_policy, inference,
    put_alibabacloud, put_custom, put_deepseek, put_elasticsearch, put_elser, put_fireworksai,
    put_hugging_face, put_llama, put_nvidia, put_openai, put_openshift_ai, put_region_policy,
    sparse_embedding, stream_completion, text_embedding, update,
};
use oxsearch::shapes::IndexMap;
use oxsearch_integration::{License, TestClient, TestIndex, client, deployed, understood};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A request the mock received: its path, its `authorization` header and
/// its body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// A mock of the OpenAI API, and of a text embeddings server under `/hf`.
struct Mock {
    /// Its URL, as the fixture reaches it.
    url: String,
    /// Its `host:port`, as the fixture reaches it.
    host: String,
    seen: Seen,
}

async fn mock() -> Mock {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let name =
        std::env::var("OXSEARCH_TEST_HOST").unwrap_or_else(|_| "host.docker.internal".to_owned());
    let seen = Seen::default();
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(serve(socket, record.clone()));
        }
    });
    Mock {
        url: format!("http://{name}:{port}"),
        host: format!("{name}:{port}"),
        seen,
    }
}

async fn serve(mut socket: TcpStream, seen: Seen) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let end = loop {
            if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
        let header = |name: &str| {
            head.lines().find_map(|line| {
                let (n, v) = line.split_once(':')?;
                n.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| v.trim().to_owned())
            })
        };
        let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let chunked = header("transfer-encoding").is_some_and(|v| v.contains("chunked"));
        let mut body = buffer.split_off(end);
        buffer.clear();
        if chunked {
            // Read until the last chunk, then take the chunks' data.
            while !body.windows(5).any(|w| w == b"0\r\n\r\n") {
                match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
            }
            body = dechunk(&body);
        } else {
            let length: usize = header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            while body.len() < length {
                match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
            }
            buffer = body.split_off(length);
        }
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let authorization = header("authorization").unwrap_or_default();
        seen.lock()
            .unwrap()
            .push((path.clone(), authorization, json.clone()));
        let (stream, status, answer) = answer(&path, &json);
        let response = if stream {
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{answer}"
            )
        } else {
            format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{answer}",
                answer.len()
            )
        };
        if socket.write_all(response.as_bytes()).await.is_err() || stream {
            return;
        }
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(line) = rest.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&rest[..line]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = line + 2;
        out.extend_from_slice(&rest[start..(start + size).min(rest.len())]);
        rest = &rest[(start + size + 2).min(rest.len())..];
    }
    out
}

/// What the mock answers: whether it's a stream, the status and the body.
fn answer(path: &str, body: &Value) -> (bool, u16, String) {
    let inputs = |field: &str| match &body[field] {
        Value::Array(items) => items.len(),
        _ => 1,
    };
    let vector = |dims: usize| (1..=dims).map(|d| 0.1 * d as f64).collect::<Vec<f64>>();
    if path.starts_with("/hf") {
        let rows: Vec<Vec<f64>> = (0..inputs("inputs")).map(|_| vector(3)).collect();
        return (false, 200, json!(rows).to_string());
    }
    if path.contains("/embeddings") {
        let dims = body["dimensions"].as_u64().unwrap_or(3) as usize;
        let data: Vec<Value> = (0..inputs("input"))
            .map(|i| json!({"object": "embedding", "index": i, "embedding": vector(dims)}))
            .collect();
        let answer = json!({"object": "list", "data": data, "model": body["model"],
            "usage": {"prompt_tokens": 2, "total_tokens": 2}});
        return (false, 200, answer.to_string());
    }
    if path.contains("/chat/completions") {
        let model = body["model"].clone();
        if body["stream"] == json!(true) {
            let chunk = |delta: Value, finish: Value| {
                json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
                    "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
            };
            let events = [
                chunk(
                    json!({"role": "assistant", "content": "Hello"}),
                    Value::Null,
                ),
                chunk(json!({"content": " there"}), Value::Null),
                chunk(json!({}), json!("stop")),
            ];
            let mut text: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
            text.push_str("data: [DONE]\n\n");
            return (true, 200, text);
        }
        let answer = json!({"id": "chatcmpl-1", "object": "chat.completion", "created": 1,
            "model": model, "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": "Hello there"}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 2, "total_tokens": 4}});
        return (false, 200, answer.to_string());
    }
    (
        false,
        404,
        json!({"error": {"message": "not found"}}).to_string(),
    )
}

/// A name no other test uses, for an endpoint.
fn name(what: &str) -> String {
    TestIndex::named(what).name().to_owned()
}

/// Deletes an endpoint the test made, if it did.
async fn forget(client: &TestClient, id: &str) {
    let _ = client.send(&delete::Request::new(id).force(true)).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn openai_through_a_mock() {
    let client = client();
    let mock = mock().await;
    let settings = |model: &str, path: &str| {
        OpenAIServiceSettings::new(model)
            .api_key("test-key")
            .url(format!("{}{path}", mock.url))
    };

    // Text embeddings, then a new key.
    let embed = name("openai-embed");
    let made = client
        .send(
            &put_openai::Request::new(OpenAITaskType::TextEmbedding, embed.as_str()).body(
                put_openai::RequestBody::new(
                    OpenAIServiceType::Openai,
                    settings("text-embedding-3-small", "/v1/embeddings").dimensions(3),
                ),
            ),
        )
        .await;
    if License::from_env() == License::Basic && matches!(made, Err(Error::License(_))) {
        return;
    }
    let made = made.unwrap();
    assert_eq!(
        made.inference_id.as_deref(),
        Some(embed.as_str()),
        "{made:?}"
    );
    let texts =
        || text_embedding::RequestBody::new(vec!["vector search".to_owned(), "hnsw".to_owned()]);
    let embedded = client
        .send(&text_embedding::Request::new(embed.as_str()).body(texts()))
        .await
        .unwrap();
    let embedded = serde_json::to_value(&embedded.0).unwrap();
    assert_eq!(
        embedded["text_embedding"].as_array().map(Vec::len),
        Some(2),
        "{embedded}"
    );
    let updated = client
        .send(
            &update::Request::new(embed.as_str())
                .task_type(TaskType::TextEmbedding)
                .body(json!({"service_settings": {"api_key": "rotated-key"}})),
        )
        .await
        .unwrap();
    assert_eq!(updated.inference_id.as_deref(), Some(embed.as_str()));
    client
        .send(&text_embedding::Request::new(embed.as_str()).body(texts()))
        .await
        .unwrap();
    let last_key = mock
        .seen
        .lock()
        .unwrap()
        .last()
        .map(|(_, key, _)| key.clone());
    assert_eq!(last_key.as_deref(), Some("Bearer rotated-key"));

    // A completion, and the same streamed.
    let complete = name("openai-complete");
    client
        .send(
            &put_openai::Request::new(OpenAITaskType::Completion, complete.as_str()).body(
                put_openai::RequestBody::new(
                    OpenAIServiceType::Openai,
                    settings("gpt-4o-mini", "/v1/chat/completions"),
                ),
            ),
        )
        .await
        .unwrap();
    let completed = client
        .send(
            &completion::Request::new(complete.as_str())
                .body(completion::RequestBody::new("Say hello".to_owned())),
        )
        .await
        .unwrap();
    assert_eq!(
        completed.completion[0].result.as_deref(),
        Some("Hello there")
    );
    let streamed = client
        .send(
            &stream_completion::Request::new(complete.as_str())
                .body(stream_completion::RequestBody::new("Say hello".to_owned())),
        )
        .await
        .unwrap();
    assert!(
        streamed.0.contains("Hello") && streamed.0.contains("there"),
        "{}",
        streamed.0
    );

    // A chat, streamed.
    let chat = name("openai-chat");
    client
        .send(
            &put_openai::Request::new(OpenAITaskType::ChatCompletion, chat.as_str()).body(
                put_openai::RequestBody::new(
                    OpenAIServiceType::Openai,
                    settings("gpt-4o-mini", "/v1/chat/completions"),
                ),
            ),
        )
        .await
        .unwrap();
    let chatted = client
        .send(&chat_completion_unified::Request::new(chat.as_str()).body(
            RequestChatCompletion::new(vec![Message::new("user").content("Say hello".to_owned())]),
        ))
        .await
        .unwrap();
    assert!(chatted.0.contains("Hello"), "{}", chatted.0);

    // Embeddings, the task for text and other content, which Elasticsearch
    // reads, and makes or refuses for this model.
    let embeddings = name("openai-embedding");
    let made = client
        .send(
            &put_openai::Request::new(OpenAITaskType::Embedding, embeddings.as_str()).body(
                put_openai::RequestBody::new(
                    OpenAIServiceType::Openai,
                    settings("text-embedding-3-small", "/v1/embeddings").dimensions(3),
                ),
            ),
        )
        .await;
    let request: RequestEmbedding =
        serde_json::from_value(json!({"input": ["vector search"]})).unwrap();
    let used = client
        .send(&embedding::Request::new(embeddings.as_str()).body(request))
        .await;
    for result in [made.map(drop), used.map(drop)] {
        if let Err(e) = result {
            assert!(understood(&e), "{e}");
        }
    }

    for id in [embed, complete, chat, embeddings] {
        forget(&client, &id).await;
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn providers_that_take_a_url() {
    let client = client();
    let mock = mock().await;
    let embeddings = format!("{}/v1/embeddings", mock.url);
    let chat = format!("{}/v1/chat/completions", mock.url);
    let ids: Vec<String> = (0..8).map(|n| name(&format!("provider-{n}"))).collect();
    let mut parser = IndexMap::new();
    parser.insert(
        "text_embeddings".to_owned(),
        "$.data[*].embedding[*]".to_owned(),
    );
    let mut secrets = IndexMap::new();
    secrets.insert("api_key".to_owned(), "test-key".to_owned());
    let results = [
        client
            .send(
                &put_deepseek::Request::new(TaskTypeDeepSeek::Completion, ids[0].as_str()).body(
                    put_deepseek::RequestBody::new(
                        DeepSeekServiceType::Deepseek,
                        DeepSeekServiceSettings::new("test-key", "deepseek-chat")
                            .url(chat.as_str()),
                    ),
                ),
            )
            .await
            .map(drop),
        client
            .send(
                &put_fireworksai::Request::new(FireworksAITaskType::TextEmbedding, ids[1].as_str())
                    .body(put_fireworksai::RequestBody::new(
                        FireworksAIServiceType::Fireworksai,
                        FireworksAIServiceSettings::new("test-key", "nomic-embed")
                            .url(embeddings.as_str())
                            .dimensions(3),
                    )),
            )
            .await
            .map(drop),
        client
            .send(
                &put_llama::Request::new(LlamaTaskType::TextEmbedding, ids[2].as_str()).body(
                    put_llama::RequestBody::new(
                        LlamaServiceType::Llama,
                        LlamaServiceSettings::new(embeddings.as_str(), "llama-embed"),
                    ),
                ),
            )
            .await
            .map(drop),
        client
            .send(
                &put_nvidia::Request::new(NvidiaTaskType::TextEmbedding, ids[3].as_str()).body(
                    put_nvidia::RequestBody::new(
                        NvidiaServiceType::Nvidia,
                        NvidiaServiceSettings::new("test-key", "nv-embed").url(embeddings.as_str()),
                    ),
                ),
            )
            .await
            .map(drop),
        client
            .send(
                &put_openshift_ai::Request::new(
                    OpenShiftAiTaskType::TextEmbedding,
                    ids[4].as_str(),
                )
                .body(put_openshift_ai::RequestBody::new(
                    OpenShiftAiServiceType::OpenshiftAi,
                    OpenShiftAiServiceSettings::new("test-key", embeddings.as_str())
                        .model_id("embed"),
                )),
            )
            .await
            .map(drop),
        client
            .send(
                &put_hugging_face::Request::new(
                    HuggingFaceTaskType::TextEmbedding,
                    ids[5].as_str(),
                )
                .body(put_hugging_face::RequestBody::new(
                    HuggingFaceServiceType::HuggingFace,
                    HuggingFaceServiceSettings::new("test-key", format!("{}/hf", mock.url)),
                )),
            )
            .await
            .map(drop),
        client
            .send(
                &put_custom::Request::new(CustomTaskType::TextEmbedding, ids[6].as_str()).body(
                    put_custom::RequestBody::new(
                        CustomServiceType::Custom,
                        CustomServiceSettings::new(
                            CustomRequestParams::new(r#"{"input": ${input}, "model": "embed"}"#),
                            CustomResponseParams::new(parser),
                            secrets,
                        )
                        .url(embeddings.as_str()),
                    ),
                ),
            )
            .await
            .map(drop),
        client
            .send(
                &put_alibabacloud::Request::new(
                    AlibabaCloudTaskType::TextEmbedding,
                    ids[7].as_str(),
                )
                .body(put_alibabacloud::RequestBody::new(
                    AlibabaCloudServiceType::AlibabacloudAiSearch,
                    AlibabaCloudServiceSettings::new(
                        "test-key",
                        mock.host.as_str(),
                        "ops-text-embedding-001",
                        "default",
                    ),
                )),
            )
            .await
            .map(drop),
    ];
    let mut made = Vec::new();
    for (n, result) in results.into_iter().enumerate() {
        match result {
            Ok(()) => made.push(n),
            Err(e) => assert!(understood(&e), "provider {n}: {e}"),
        }
    }
    eprintln!("made: {made:?} of the eight providers");
    for id in &ids {
        forget(&client, id).await;
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn elasticsearch_service() {
    let client = client();
    if License::from_env() == License::Basic {
        let refused = client
            .send(
                &put_elser::Request::new(ElserTaskType::SparseEmbedding, name("elser").as_str())
                    .body(put_elser::RequestBody::new(
                        ElserServiceType::Elser,
                        ElserServiceSettings::new(1, 1),
                    )),
            )
            .await;
        assert!(matches!(&refused, Err(e) if understood(e)), "{refused:?}");
        return;
    }
    // The default ELSER endpoint's deployment, started if it isn't.
    let warm = sparse_embedding::Request::new(".elser-2-elasticsearch")
        .body(sparse_embedding::RequestBody::new("warm up".to_owned()));
    deployed(|| client.send(&warm)).await;
    let default = client
        .send(&get::Request::new().inference_id(".elser-2-elasticsearch"))
        .await
        .unwrap();
    let model = default.endpoints[0]
        .service_settings
        .as_ref()
        .and_then(|s| s["model_id"].as_str())
        .expect("the default endpoint's model")
        .to_owned();

    // An endpoint of our own on that deployment.
    let ours = name("elasticsearch-service");
    client
        .send(
            &put_elasticsearch::Request::new(ElasticsearchTaskType::SparseEmbedding, ours.as_str())
                .body(put_elasticsearch::RequestBody::new(
                    ElasticsearchServiceType::Elasticsearch,
                    ElasticsearchServiceSettings::new(model)
                        .deployment_id(".elser-2-elasticsearch"),
                )),
        )
        .await
        .unwrap();
    let embedded = client
        .send(
            &inference::Request::new(ours.as_str())
                .body(inference::RequestBody::new(vec!["replicas".to_owned()])),
        )
        .await
        .unwrap();
    let embedded = serde_json::to_value(&embedded.0).unwrap();
    assert!(embedded["sparse_embedding"].is_array(), "{embedded}");
    forget(&client, &ours).await;

    // ELSER's own service, which makes a deployment of its own: made or
    // refused, depending on the memory left.
    let elser = name("elser");
    let made = client
        .send(
            &put_elser::Request::new(ElserTaskType::SparseEmbedding, elser.as_str()).body(
                put_elser::RequestBody::new(
                    ElserServiceType::Elser,
                    ElserServiceSettings::new(1, 1),
                ),
            ),
        )
        .await;
    if let Err(e) = made {
        assert!(understood(&e), "{e}");
    }
    forget(&client, &elser).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn region_policy() {
    let client = client();
    let results = [
        client
            .send(
                &put_region_policy::Request::new().body(put_region_policy::RequestBody::new(
                    RegionPolicy::AllowedGeos(vec!["eu".to_owned()]),
                )),
            )
            .await
            .map(drop),
        client
            .send(&get_region_policy::Request::new())
            .await
            .map(drop),
        client
            .send(&delete_region_policy::Request::new())
            .await
            .map(drop),
    ];
    for result in results {
        if let Err(e) = result {
            assert!(understood(&e), "{e}");
        }
    }
}
