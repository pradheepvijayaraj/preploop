//! Build the bundled Granite R2 vector generation from the UPSC corpus.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_lib::search::embedding::engine::EmbeddingEngine;
use app_lib::search::embedding::llama_cpp::LlamaCppEmbeddingEngine;
use app_lib::search::indexing::fingerprint::content_fingerprint;
use app_lib::search::indexing::generation::GenerationManager;
use app_lib::search::vector::format::VectorRecord;
use serde::{Deserialize, Serialize};

const MODEL_REVISION: &str = "2ab6fa8ea2d674564defd37171ae19079b864b33";
const EMBEDDING_BATCH_SIZE: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    content_version: u64,
    papers: Vec<CatalogPaper>,
}

#[derive(Debug, Deserialize)]
struct CatalogPaper {
    path: String,
}

#[derive(Debug, Deserialize)]
struct QuestionBank {
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
struct Question {
    id: String,
    question: String,
    #[serde(default)]
    options: Vec<QuestionOption>,
}

#[derive(Debug, Deserialize)]
struct QuestionOption {
    id: String,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundledIndexMetadata<'a> {
    model: &'a str,
    model_revision: &'a str,
    dimensions: usize,
    pooling: &'a str,
    quantization: &'a str,
    content_version: u64,
    question_count: usize,
    question_ids: Vec<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("Read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Parse {}: {error}", path.display()))
}

fn canonical_text(question: &Question) -> String {
    let options = question
        .options
        .iter()
        .map(|option| format!("({}) {}", option.id, option.text.trim()))
        .collect::<Vec<_>>()
        .join(" ");
    if options.is_empty() {
        question.question.clone()
    } else {
        format!("{}\n{}", question.question, options)
    }
}

fn main() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus_root = manifest_dir.join("../static/upsc");
    let catalog: Catalog = read_json(&corpus_root.join("catalog.json"))?;
    let model_path = manifest_dir.join("models/granite-r2-q8_0.gguf");
    let output_dir = manifest_dir.join("models/search-index");

    let mut granite = LlamaCppEmbeddingEngine::new(&model_path)
        .map_err(|error| format!("Initialise Granite: {error}"))?;
    let accelerated = std::env::var("PREPLOOP_INDEX_GPU").is_ok_and(|value| value == "1");
    if accelerated {
        granite = granite.with_n_gpu_layers(u32::MAX);
    }
    let engine: Arc<dyn EmbeddingEngine> = Arc::new(granite);

    let mut question_ids = Vec::new();
    let mut texts = Vec::new();
    for paper in &catalog.papers {
        let bank: QuestionBank = read_json(&corpus_root.join(&paper.path))?;
        for question in bank.questions {
            question_ids.push(question.id.clone());
            texts.push(canonical_text(&question));
        }
    }

    println!(
        "Embedding {} questions with Granite R2 Q8_0 on {}",
        texts.len(),
        if accelerated {
            "the available GPU"
        } else {
            "CPU"
        }
    );
    let mut records = Vec::with_capacity(texts.len());
    for (batch_index, chunk) in texts.chunks(EMBEDDING_BATCH_SIZE).enumerate() {
        let embeddings = engine
            .embed_documents(chunk)
            .map_err(|error| format!("Embedding batch {batch_index}: {error}"))?;
        for (offset, embedding) in embeddings.iter().enumerate() {
            let index = batch_index * EMBEDDING_BATCH_SIZE + offset;
            records.push(VectorRecord::from_embedding(
                index as u64 + 1,
                content_fingerprint(&texts[index]),
                0,
                embedding,
            )?);
        }
        println!("Embedded {}/{}", records.len(), texts.len());
    }

    GenerationManager::build_and_swap_generation(
        &output_dir,
        1,
        MODEL_REVISION,
        catalog.content_version,
        &records,
    )?;

    let metadata = BundledIndexMetadata {
        model: "ibm-granite/granite-embedding-small-english-r2",
        model_revision: MODEL_REVISION,
        dimensions: engine.dimensions(),
        pooling: "CLS",
        quantization: "Q8_0 model / int8 vectors",
        content_version: catalog.content_version,
        question_count: records.len(),
        question_ids,
    };
    let generation_dir = output_dir.join("generation-001");
    fs::write(
        generation_dir.join("questions.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Write bundled metadata: {error}"))?;

    println!(
        "Wrote {} records to {}",
        records.len(),
        generation_dir.display()
    );
    Ok(())
}
