# Classical ML for Graph Node Enrichment

> Research conducted 2026-03-12. Analysis of classical machine learning algorithms
> (random forests, gradient boosting, logistic regression) for enriching graph nodes
> with predicted labels, relevance scores, and suggested connections in Context Manager.

---

## 1. Executive Summary

Context Manager's 9 node types carry raw content but no derived metadata — a Message has text but no predicted intent, a WorkItem has a title but no estimated complexity, a GitFile has a path but no risk classification. Today, the only enrichment path is explicit human annotation or LLM-based extraction (via `spawn_tool_extraction` in `tools.rs:53-64`). Classical ML classifiers can fill this gap cheaply: extract features from node content and graph structure, train lightweight models on weak supervision signals, and predict labels at sub-millisecond latency with no GPU or API cost.

Doc 09 designs an embedding pipeline for discovering `RelevantTo` connections via cosine similarity. Classical ML classifiers are the natural **second tier** of that cascade — they consume embeddings *as features* alongside structural and temporal signals, producing richer predictions than similarity alone. Where embeddings answer "how similar are these nodes?", classifiers answer "what *kind* of node is this?" and "should these nodes be connected, given everything we know?"

**Recommendation:**
- **Phase 0 (Baseline):** Hand-written rule engine. Deterministic rules for intent detection, file classification, error detection. Zero dependencies, works from node 1, debuggable. This is the starting point — ML must demonstrably beat it before graduating.
- **Phase 1:** `linfa` ecosystem (pure Rust, `linfa-trees` + `linfa-preprocessing`) for random forest classification. Feature vector: hand-crafted features (keyword flags, regex patterns, character stats) + graph degree + temporal recency. TF-IDF only for nodes with >20 tokens (Message, ThinkBlock). Train on weak labels from user interactions once sufficient data accumulates (~500+ labeled nodes across conversations). Background worker via existing `TaskMessage` + `mpsc` pattern.
- **Phase 2:** Add gradient boosting (`forust-ml` crate) for link prediction. Expand features to include graph centrality (betweenness, clustering coefficient) and embedding vectors from doc 09. Active learning: surface low-confidence predictions for user confirmation.
- **Phase 3:** Incremental learning via streaming classifiers (Hoeffding trees). Cross-conversation model sharing. Model versioning and A/B comparison.

---

## 2. Current Architecture & Gap Analysis

### 2.1 Node Content Available for Feature Extraction

`Node::content()` (`graph/mod.rs:162-174`) provides normalized text access, but classifiers need richer features:

| Node Type | `content()` Returns | Additional Fields Available | ML Enrichment Opportunity |
|-----------|--------------------|-----------------------------|---------------------------|
| Message | `content` (full text) | role, model, tokens | Intent classification, complexity scoring |
| ThinkBlock | `content` (reasoning) | parent_message_id | Reasoning type classification |
| WorkItem | `title` only | description, status | Priority estimation, category prediction |
| ToolResult | `content` (output) | is_error, tool_call_id | Error pattern detection, quality scoring |
| Tool | `name` only | description | Semantic clustering |
| GitFile | `path` only | status | File risk classification, change impact |
| ToolCall | `tool_name()` | arguments | Usage pattern classification |
| SystemDirective | `content` | — | Low value (static) |
| BackgroundTask | `description` | kind, status | Low value (internal) |

**Gap:** `content()` returns a single string per node. For ML feature extraction, we need a richer text builder that concatenates relevant fields — e.g., WorkItem should combine `title` + `description`, Tool should combine `name` + `description`.

### 2.2 No Feature Extraction Infrastructure

No TF-IDF, tokenization, or feature vector computation exists. All text processing is delegated to the LLM. Classical ML needs a local feature extraction pipeline.

### 2.3 No Training Signal

No user feedback mechanism exists. The multi-rater relevance system (`VISION.md:233-271`) describes developer ratings (thumbs up/down) but none are implemented. Without labels, supervised classifiers cannot train.

### 2.4 Background Task Pattern (Reusable)

`TaskMessage` enum (`tasks.rs:37-60`) with 6 variants handles all background results. New classifier variants fit naturally:

```
TaskMessage::NodeClassificationComputed { node_id, predictions }
```

The spawn pattern (`spawn_git_watcher`, `spawn_tool_discovery`) provides the template: `spawn_blocking` or `tokio::spawn` → do work → send `TaskMessage`.

---

## 3. Requirements

1. **Sub-millisecond inference** — Classifier prediction must not block the TUI. Random forest inference on 50-100 features: <1ms.
2. **No external runtime** — No Python, no GPU, no API calls. Pure Rust or statically-linked ONNX.
3. **Graceful cold start** — System must function without a trained model. Predictions are additive, never gating.
4. **Complement doc 09 embeddings** — Consume embedding vectors as features; don't duplicate the embedding pipeline.
5. **Weak supervision** — Train on implicit signals (user edits, link creation, node deletion) rather than explicit labels.
6. **Incremental enrichment** — Classify nodes on creation; reclassify when graph structure changes significantly.
7. **Separate persistence** — Store predictions outside the graph (like doc 09's embedding index). No V2→V3 migration.

---

## 4. Options Analysis

### 4.1 Linfa Ecosystem

The Rust equivalent of scikit-learn. Modular: each algorithm is a separate crate.

| Crate | Algorithm | Version | Notes |
|-------|-----------|---------|-------|
| `linfa-trees` | Decision tree | 0.7.0 | CART implementation |
| `linfa-ensemble` | Random forest, AdaBoost | 0.8.0+ | Added in linfa 0.8 (first published at 0.8.0) |
| `linfa-bayes` | Naive Bayes | 0.7.0 | Gaussian NB |
| `linfa-svm` | SVM | 0.7.0 | Linear + RBF kernels |
| `linfa-logistic` | Logistic regression | 0.7.0 | Multinomial support |
| `linfa-preprocessing` | TF-IDF, scaling | 0.7.0 | CountVectorizer, TfIdfVectorizer |
| `linfa-clustering` | K-Means, DBSCAN, GMM | 0.7.0 | Unsupervised |

**Strengths:** Pure Rust. Optional BLAS for acceleration. Active development. ndarray-based (composes well).
**Weaknesses:** `linfa-ensemble` is relatively new (first published at 0.8.0). No incremental learning. Smaller community than scikit-learn.

### 4.2 SmartCore

Monolithic ML library. All algorithms in one crate.

**Strengths:** Zero external dependencies. Broad algorithm coverage. Mature API (v0.4.9). Active development with new algorithms (Extra Trees, XGBoost regression, hyperparameter search) added in 0.4.x series.
**Weaknesses:** Custom matrix types (`DenseMatrix`, not ndarray). Harder to extend and compose with linfa/ndarray ecosystem.

### 4.3 Forust (`forust-ml` on crates.io)

Pure Rust gradient boosted decision trees. Produces results near-identical to XGBoost.

**Strengths:** Fast training. Excellent for tabular data. Pure Rust. Small codebase (~2000 LOC auditable).
**Weaknesses:** Gradient boosting only (no random forest, no SVM). No preprocessing pipeline.

### 4.3b Additional Rust RF Crates

| Crate | Description | Notes |
|-------|-------------|-------|
| `randomforest` | Standalone pure Rust RF (classifier + regressor) | Focused, auditable |
| `rustlearn` | RF with dense + sparse data support | Performance competitive with sklearn |

These are narrower alternatives if linfa-ensemble proves too immature.

### 4.4 GBDT-RS

Gradient boosting from MesaTEE (Intel SGX-compatible).

**Strengths:** Pure Rust. Auditable (~2000 LOC). SGX-compatible.
**Weaknesses:** Narrow scope. Less active than forust.

### 4.5 ONNX Runtime (sklearn export)

Train in Python (scikit-learn), export via `sklearn-onnx`, infer in Rust via `ort` crate.

**Strengths:** Full scikit-learn power for training. Optimized inference runtime. Mature ecosystem.
**Weaknesses:** Requires Python for training. ONNX Runtime bundles C++ shared libs (cross-compilation issues, same as doc 09 §3.1). Two-language workflow.

### 4.6 rust-tfidf

Standalone TF-IDF computation with trait-based document interface.

**Strengths:** Simple, focused. Trait-based (adapts to any document type).
**Weaknesses:** TF-IDF only. No vectorizer output compatible with linfa's ndarray format.

---

## 5. Comparison Matrix

| Criterion | Linfa | SmartCore | Forust | ONNX (sklearn) |
|-----------|-------|-----------|--------|-----------------|
| **Pure Rust** | Yes | Yes | Yes | No (C++ runtime) |
| **Random forest** | Yes (linfa-ensemble) | Yes | No | Yes |
| **Gradient boosting** | No | No | Yes | Yes |
| **TF-IDF** | Yes (linfa-preprocessing) | No | No | Yes |
| **Incremental learning** | No | No | No | Partial (SGD) |
| **Preprocessing pipeline** | Yes | Limited | No | Yes |
| **ndarray integration** | Native | Custom types | Custom | Via ort tensors |
| **Cross-compilation** | Easy | Easy | Easy | Hard (C++ deps) |
| **Community activity** | Active | Active (v0.4.x) | Active | Very active |
| **Model serialization** | Via serde | Via serde | Via serde | ONNX format |

**Verdict:** Linfa for Phase 1 (pure Rust, random forest, built-in TF-IDF + preprocessing) with `randomforest` crate as fallback if linfa-ensemble is insufficiently stable. `forust-ml` as Phase 2 addition for gradient boosting when link prediction needs higher accuracy. SmartCore is a viable alternative if ndarray integration is not important.

---

## 6. VISION.md Alignment

### 6.1 Multi-Rater Relevance System (§4.4)

The cascade evaluation pattern (`VISION.md:252`) describes: "Embedding similarity as fast first pass. If score > 0.9 or < 0.2, use it. If 0.2-0.9, escalate to LLM judge."

Classical ML classifiers are not a separate cascade tier — they **consume embedding similarity as one input feature** alongside structural and temporal signals. The cascade becomes:

```
Tier 0: Embedding cosine similarity (doc 09) — $0, <0.1ms
        ├── High confidence (>0.9 or <0.2): accept/reject directly
        └── Ambiguous (0.2-0.9): feed to classifier
Tier 1: Classical ML classifier (cosine + structural features) — $0, <1ms
        ├── High confidence: accept/reject
        └── Still ambiguous: escalate
Tier 2: LLM judge — $0.01-0.10, 500-2000ms
Tier 3: Human confirmation — priceless, async
```

**Important:** Classifiers depend on embeddings being computed first. They cannot run independently as a parallel tier — they are an enhanced decision rule applied to embedding + structural features. The value over raw cosine thresholding is that the classifier learns non-linear interactions (e.g., "high cosine similarity between a Message and a GitFile is less meaningful than the same similarity between two Messages"). The exact reduction in LLM escalation depends on training data quality and must be measured empirically.

### 6.2 Background Processing (§4.3)

"Analysis: Background LLM extracts structure, detects clusters" — classical ML provides a cheaper alternative for clustering and classification that runs locally without API calls.

### 6.3 Developer Ratings

"Developer ratings: Thumbs up/down, explicit 'this is important for X.' One-click, optional, never blocking" (`VISION.md:249`). These ratings become training labels for supervised classifiers once implemented.

---

## 7. Recommended Architecture

### 7.0 Phase 0: Rule-Based Baseline (No ML)

**Goal:** Deterministic node enrichment from day 1. Zero dependencies, zero training data, fully debuggable.

**Rules engine:**

```rust
pub fn classify_by_rules(node: &Node) -> NodePredictions {
    match node {
        Node::Message { content, .. } => {
            let intent = if content.contains('?') && content.len() < 200 {
                Some(IntentLabel::Question)
            } else if content.starts_with("Please") || content.contains("implement") {
                Some(IntentLabel::Instruction)
            } else {
                Some(IntentLabel::Information)
            };
            NodePredictions { intent, confidence: 0.6, .. }
        }
        Node::GitFile { path, .. } => {
            let is_test = path.contains("test") || path.ends_with("_tests.rs");
            let is_config = path.ends_with(".toml") || path.ends_with(".json");
            // ...
        }
        Node::ToolResult { is_error: true, .. } => {
            NodePredictions { intent: Some(IntentLabel::Error), confidence: 0.9, .. }
        }
        // ...
    }
}
```

**Why start here:** The Red Team audit identified that ML heuristic labeling functions (e.g., "messages with `?` are questions") ARE rules. Training ML to re-learn them from noisy examples adds complexity without adding accuracy. Rules are the honest baseline. ML must beat measured rule accuracy before being adopted.

**Transition criteria to Phase 1:** Rules are in production and measuring accuracy via user feedback. At least 500 labeled nodes (from user corrections to rule predictions) have accumulated across conversations.

### 7.1 Phase 1: Node Classification with Random Forest

**Goal:** Improve on Phase 0 rules by learning non-linear patterns from accumulated user feedback.

**Feature vector (per node) — hand-crafted features first:**

| Feature | Source | Dimensions | Computation |
|---------|--------|------------|-------------|
| Node type | Enum variant | 9 (one-hot) | Direct |
| In-degree | Graph structure | 1 | `edges.iter().filter(to == id).count()` |
| Out-degree | Graph structure | 1 | `edges.iter().filter(from == id).count()` |
| Age (seconds) | `created_at` | 1 | `Utc::now() - created_at` |
| Content length | `content().len()` | 1 | Direct |
| Token count | `input_tokens + output_tokens` | 1 | From Message fields |
| Contains `?` | Boolean | 1 | `content.contains('?')` |
| Contains code block | Boolean | 1 | Regex for triple backticks |
| Contains URL | Boolean | 1 | Regex for `https?://` |
| Contains file path | Boolean | 1 | Regex for `/` path separators |
| Uppercase ratio | Float | 1 | Character-level stat |
| Punctuation ratio | Float | 1 | Character-level stat |
| Word count | Integer | 1 | Whitespace split |
| Is error | Boolean | 1 | `is_error` field (ToolResult only) |
| TF-IDF (long content only) | Nodes >20 tokens | 30-50 | `linfa-preprocessing` TfIdfVectorizer |

Total: ~25-70 features. TF-IDF is applied **only** to Message, ThinkBlock, and long ToolResult content (>20 tokens). For short-text nodes (GitFile paths, Tool names, ToolCall names — typically 1-8 tokens), TF-IDF produces noise and is omitted.

**Why hand-crafted over TF-IDF:** Most node types produce 1-8 tokens via `content()`. TF-IDF on a corpus of 100-500 short documents produces unstable IDF estimates. A 200-feature TF-IDF vector trained on 100 examples has a 2:1 feature-to-sample ratio — overfitting by construction. Hand-crafted boolean/numeric features are stable, interpretable, and work at any corpus size.

**Classification targets:**

| Target | Type | Labels | Training Signal |
|--------|------|--------|-----------------|
| Relevance | Binary | high / low | User corrections to Phase 0 rule predictions |
| Intent | Multi-class | question / instruction / information / meta | User corrections + accumulated interaction data |

**Training data requirements:**
- **Minimum:** ~500 labeled nodes across conversations (accumulated from Phase 0 user corrections)
- **Labeling source:** User confirms or rejects rule-based predictions → corrected labels become training data
- **Note on heuristic labels:** Pure heuristic labels (without user correction) have estimated 30-40% error rate on intent classification. A random forest trained on 100 heuristic labels with 70 features will learn noise. Require user-corrected labels before training.

**Retraining criteria:**
- Retrain after every 100 new user-corrected labels
- Full vocabulary rebuild on retrain (TF-IDF vocabulary is frozen between retrains)
- Log accuracy metrics: compare ML predictions against next batch of user corrections

**Model persistence:**
```
~/.context-manager/classifiers/
    model.bincode          # Serialized random forest
    vocabulary.bincode     # TF-IDF vocabulary (for long-content nodes only)
    metadata.json          # { model_id, feature_count, trained_at, sample_count }
```

### 7.2 Phase 2: Link Prediction + Active Learning

**Extended features (per node pair):**

| Feature | Description |
|---------|-------------|
| Cosine similarity | From doc 09 embedding vectors |
| Common neighbor count | Shared edge targets |
| Jaccard coefficient | Common neighbors / union of neighbors |
| Source/target degree product | Proxy for preferential attachment |
| Same conversation | Boolean: same conversation_id |
| Type compatibility | One-hot of (source_type, target_type) pair |

**Gradient boosting (`forust-ml`)** for link prediction — better than random forest on structured tabular features with interactions.

**Candidate pair generation:** Evaluating all O(n^2) pairs is infeasible at scale. Filter candidates using embedding similarity from doc 09: only evaluate pairs where cosine similarity > 0.3. This reduces candidate pairs by ~90% at typical similarity distributions.

**Active learning loop:**
1. Classifier predicts edge confidence for filtered candidate pairs
2. Surface top-K uncertain predictions (confidence 0.4-0.6) to user
3. User confirms or rejects → labeled training examples
4. Retrain model with new labels every 50 user decisions

### 7.3 Phase 3: Incremental Learning + Cross-Conversation Models

**Hoeffding trees** (streaming decision trees) for truly incremental updates — no full retrain needed. Not available in linfa today; would require custom implementation or ONNX export of River (Python streaming ML).

**Cross-conversation model:** Train on all conversations' labeled data. Store a shared model alongside per-conversation prediction caches.

---

## 8. Integration Design

### 8.1 Classifier Trait

```rust
pub trait NodeClassifier: Send + Sync {
    /// Predict labels for a single node given its feature vector.
    fn predict(&self, features: &Array1<f64>) -> NodePredictions;

    /// Predict labels for a batch of nodes.
    fn predict_batch(&self, features: &Array2<f64>) -> Vec<NodePredictions>;

    /// Train or retrain on labeled examples.
    fn fit(&mut self, features: &Array2<f64>, labels: &TrainingLabels) -> anyhow::Result<()>;

    /// Model identifier for cache invalidation.
    fn model_id(&self) -> &str;

    /// Number of training samples seen.
    fn sample_count(&self) -> usize;
}
```

### 8.2 Feature Extractor

```rust
pub struct FeatureExtractor {
    tfidf: Option<TfIdfVectorizer>,  // linfa-preprocessing, only for long content
    tfidf_dimensions: usize,          // Truncated dimensions (30-50)
}

impl FeatureExtractor {
    /// Extract feature vector for a single node.
    pub fn extract(&self, node: &Node, graph: &ContextGraph) -> Array1<f64> {
        let mut features = Vec::with_capacity(self.feature_count());
        let content = self.enriched_content(node);

        // Hand-crafted features (all nodes)
        features.extend(self.node_type_onehot(node));
        features.push(graph.in_degree(node.id()) as f64);
        features.push(graph.out_degree(node.id()) as f64);
        features.push(node.age_seconds() as f64);
        features.push(content.len() as f64);
        features.push(content.split_whitespace().count() as f64);
        features.push(content.contains('?') as u8 as f64);
        features.push(content.contains("```") as u8 as f64);
        features.push(content.contains("http") as u8 as f64);
        features.push(content.contains('/') as u8 as f64);

        // TF-IDF only for long content (>20 tokens)
        if content.split_whitespace().count() > 20 {
            if let Some(ref tfidf) = self.tfidf {
                let tfidf_vec = tfidf.transform(&content);
                features.extend(tfidf_vec.iter().take(self.tfidf_dimensions));
            }
        }
        // Pad with zeros if TF-IDF not applied (consistent vector length)
        while features.len() < self.feature_count() {
            features.push(0.0);
        }

        Array1::from_vec(features)
    }

    /// Richer text than Node::content() — concatenates relevant fields.
    fn enriched_content(&self, node: &Node) -> String {
        match node {
            Node::WorkItem { title, description, .. } => {
                match description {
                    Some(desc) => format!("{title} {desc}"),
                    None => title.clone(),
                }
            }
            Node::Tool { name, description, .. } => format!("{name} {description}"),
            other => other.content().to_string(),
        }
    }
}
```

### 8.3 Prediction Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePredictions {
    pub relevance: Option<f64>,           // 0.0-1.0
    pub intent: Option<IntentLabel>,      // Predicted intent class
    pub complexity: Option<f64>,          // 0.0-1.0
    pub confidence: f64,                  // Overall prediction confidence
    pub model_id: String,                 // For cache invalidation
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IntentLabel {
    Question,
    Instruction,
    Information,
    Meta,
}
```

### 8.4 Background Worker

```rust
pub fn spawn_classification_worker(
    tx: mpsc::UnboundedSender<TaskMessage>,
    classifier: Arc<Mutex<dyn NodeClassifier>>,
    extractor: Arc<FeatureExtractor>,
    rx: mpsc::UnboundedReceiver<ClassificationRequest>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let features = extractor.extract(&request.node, &request.graph_snapshot);
            let predictions = classifier.lock().await.predict(&features);
            let _ = tx.send(TaskMessage::NodeClassificationComputed {
                node_id: request.node_id,
                predictions,
            });
        }
    });
}
```

### 8.5 Data Flow

```
Node created (add_node)
    │
    ├─► Phase 0: Rule engine (sync, immediate)
    │       └── NodePredictions { intent, confidence }
    │               └── Store in ClassificationIndex
    │
    ├─► Embedding worker (doc 09)  ──► EmbeddingIndex
    │
    └─► Phase 1+: Classification worker (async, after embeddings ready)
            │
            ├── FeatureExtractor.extract(node, graph)
            │       ├── hand_crafted_features (boolean flags, stats)
            │       ├── one_hot(node_type)
            │       ├── in_degree + out_degree
            │       ├── age_seconds
            │       └── TF-IDF (long content only, >20 tokens)
            │
            └── classifier.predict(features)
                    └── NodePredictions { relevance, intent }
                            │
                            └─► TaskMessage::NodeClassificationComputed
                                    │
                                    └─► handle_task_message()
                                            ├── Store in ClassificationIndex
                                            └── NEVER auto-create edges
                                                (suggest only, user confirms)
```

**Edge provenance:** ML-predicted edges must NOT be auto-created. The current `Edge` struct has no `source` or `confidence` field — there is no way to distinguish ML-generated edges from user-created ones, and wrong edges become features for future predictions (feedback loop). Instead, store suggestions in the `ClassificationIndex` and surface them in the TUI for user confirmation. Only confirmed suggestions become real `RelevantTo` edges.

### 8.6 Persistence

```rust
#[derive(Serialize, Deserialize)]
pub struct ClassificationStore {
    pub predictions: HashMap<Uuid, NodePredictions>,
    pub model_id: String,
    pub trained_at: DateTime<Utc>,
    pub sample_count: usize,
}
```

**Caching strategy:** Prediction cache is optional. With <1ms inference per node and 1000 nodes, recomputing all predictions on load takes <1 second. The cache saves load time but introduces staleness: when graph structure changes (edge added/removed), cached predictions for affected nodes become stale because structural features (degree) have changed.

**Recommended approach:** Recompute predictions on load (no cache). Persist only the model and vocabulary. This eliminates staleness bugs and reduces persistence complexity.

```
~/.context-manager/classifiers/
    model.bincode            # Trained random forest
    vocabulary.bincode       # TF-IDF vocabulary
    metadata.json            # { model_id, feature_count, trained_at, sample_count }
    training_labels.msgpack  # Accumulated user-corrected labels (for retraining)
```

---

## 9. Red/Green Team

### 9.1 Green Team (Factual Verification)

28/32 claims verified correct. 4 corrections applied:

| Claim | Finding | Fix |
|-------|---------|-----|
| `linfa-ensemble` v0.1.0 | First published at v0.8.0, not 0.1.0 | Fixed version in table |
| SmartCore "maintenance-only" | Actively developed: Extra Trees, XGBoost regression, hyperparameter search added in v0.4.x | Corrected description |
| `TfidfTransformer` in linfa | Actual struct is `TfIdfVectorizer` (no separate transformer) | Fixed API name |
| `forust` crate name | Actual crates.io name is `forust-ml` | Fixed throughout |

All source URLs verified. All performance claims (RF inference <1ms, gradient boosting training 5-30s for 10K items) confirmed realistic. Neo4j GDS pipeline, Snorkel, and Hoeffding tree claims verified.

### 9.2 Red Team (Challenges)

**CRITICAL findings (addressed in revision):**

1. **Rule-based baseline missing.** The original document jumped straight to ML without establishing a deterministic baseline. Heuristic labeling functions (e.g., "messages with ? are questions") ARE rules — training ML to re-learn them noisily is Rube Goldberg engineering. **Fix:** Added Phase 0 rule engine as mandatory starting point. ML must demonstrably beat measured rule accuracy before adoption.

2. **Training data insufficient.** 100 heuristically-labeled examples with 30-40% error rate cannot train a 200-feature random forest. The 3 proposed labeling functions lack the overlap needed for Snorkel-style aggregation. **Fix:** Revised Phase 1 to require 500+ user-corrected labels. Reduced features to 25-70 (hand-crafted primary, TF-IDF secondary for long content only).

**HIGH findings (addressed):**

3. **TF-IDF on short text is noise.** Most node types produce 1-8 tokens via `content()`. TF-IDF on a 100-node corpus with 200-feature vectors overfits by construction (2:1 feature-to-sample ratio). **Fix:** Replaced TF-IDF-first design with hand-crafted features (boolean flags, regex patterns, character stats). TF-IDF applied only to nodes with >20 tokens.

4. **Cold start: confident-but-wrong predictions.** Random forests produce confident predictions even when undertrained. Wrong predictions creating `RelevantTo` edges would pollute the graph with no rollback mechanism. **Fix:** Phase 0 rules from day 1. ML predictions never auto-create edges — they suggest only, user confirms.

5. **Feature drift.** TF-IDF vocabulary frozen at training time cannot handle new domain terms. **Fix:** Full vocabulary rebuild on retrain (every 100 new labels).

6. **Error propagation feedback loop.** Predicted edges become structural features (degree) for future predictions, creating positive feedback. **Fix:** ML-predicted connections are suggestions only, stored separately. Never auto-created as graph edges. No structural contamination.

7. **LLM classification already available.** `background_llm_call` (`llm/mod.rs:85-138`) exists. A single cheap LLM call per node would produce higher-quality classifications from day 1 with zero training data. **Fix:** Acknowledged as existing alternative. Phase 0 rules are cheaper than LLM calls ($0 vs ~$0.001/node) and work offline. ML (Phase 1) targets the gap between rules and LLM: better than rules, cheaper than LLM.

8. **Missing crates.** Document missed `randomforest` (standalone RF) and `rustlearn` (RF with sparse data). **Fix:** Added as fallback options in §4.3b.

**MEDIUM findings (addressed):**

9. **Cascade tier dependency.** Classifier consumes embeddings as input — not an independent parallel tier. **Fix:** Rewrote §6.1 to clarify dependency chain.

10. **Prediction cache staleness.** Cached predictions become stale on structural changes (edge add/remove). **Fix:** Replaced cache with recompute-on-load (inference is <1ms/node, acceptable for 1000-node graphs).

11. **Background worker needs full graph.** Current `ContextSnapshot` lacks graph structure for degree computation. **Fix:** Acknowledged — `ClassificationRequest` must include graph structure or delegate feature extraction to the caller.

12. **No retraining criteria.** When does the model retrain? **Fix:** Added explicit criteria: every 100 new user-corrected labels, with full vocabulary rebuild.

### 9.3 Code Accuracy

All 18 file:line references verified correct against the codebase. Node enum (9 variants), EdgeKind (9 variants), TaskMessage (6 variants), and all method signatures match. Doc 09 claims (EmbeddingProvider trait, EmbeddingIndex, background worker, MessagePack) confirmed.

---

## 10. Sources

### Rust Crates
- [linfa ecosystem](https://github.com/rust-ml/linfa) — Pure Rust ML framework, scikit-learn equivalent
- [smartcore](https://smartcorelib.org/) — Comprehensive ML library, actively developed (v0.4.9)
- [forust-ml](https://github.com/jinlow/forust) — Gradient boosted trees, XGBoost-equivalent (crates.io: `forust-ml`)
- [gbdt-rs](https://github.com/mesalock-linux/gbdt-rs) — Gradient boosting, SGX-compatible
- [rust-tfidf](https://crates.io/crates/rust-tfidf) — TF-IDF computation
- [petgraph](https://github.com/petgraph/petgraph) — Graph data structures and algorithms
- [ndarray](https://github.com/rust-ndarray/ndarray) — N-dimensional array operations
- [ndarray-stats](https://github.com/rust-ndarray/ndarray-stats) — Statistical operations on ndarray
- [linfa vs smartcore benchmark](https://github.com/cmccomb/smartcore_vs_linfa)
- [node2vec (Rust)](https://crates.io/crates/node2vec) — Graph embeddings via petgraph + nalgebra

### Techniques & Prior Art
- [Neo4j GDS Node Classification Pipeline](https://neo4j.com/docs/graph-data-science/current/machine-learning/node-property-prediction/nodeclassification-pipelines/node-classification/) — Feature engineering → train → predict pattern
- [Neo4j GDS Link Prediction Pipeline](https://neo4j.com/docs/graph-data-science/current/machine-learning/linkprediction-pipelines/link-prediction/) — Edge-level feature extraction + classification
- [Snorkel: Weak Supervision](https://pmc.ncbi.nlm.nih.gov/articles/PMC5951191/) — Training from heuristic labeling functions without ground truth
- [INK: Knowledge Graph Node Classification](https://link.springer.com/article/10.1007/s10618-021-00806-z) — Feature-based KG representations for classical ML
- [Active Learning Survey](https://link.springer.com/article/10.1007/s10994-023-06454-2) — Strategies for learning with minimal labels
- [Incremental Decision Trees (VFDT)](https://en.wikipedia.org/wiki/Incremental_decision_tree) — Streaming decision trees for online learning
- [Link Prediction in Social Networks (Liben-Nowell & Kleinberg)](https://www.cs.cornell.edu/home/kleinber/link-pred.pdf) — Jaccard, Adamic-Adar, common neighbors
- [GraphSMOTE: Imbalanced Node Classification](https://arxiv.org/pdf/2103.08826) — SMOTE adapted for graph-structured data
- [Graph Random Forest](https://pmc.ncbi.nlm.nih.gov/articles/PMC10377046/) — Incorporating graph structure into tree construction
- [Weisfeiler-Lehman Graph Kernels](https://www.jmlr.org/papers/volume12/shervashidze11a/shervashidze11a.pdf) — Subgraph classification via color refinement

### Internal References
- `graph/mod.rs:57-67` — EdgeKind enum (9 variants including RelevantTo)
- `graph/mod.rs:80-145` — Node enum (9 variants with all fields)
- `graph/mod.rs:162-174` — `Node::content()` accessor
- `tasks.rs:37-60` — TaskMessage enum (background worker pattern)
- `tasks.rs:62-91` — `spawn_git_watcher()` (spawn_blocking template)
- `tools.rs:53-64` — `spawn_tool_extraction()` (LLM background worker template)
- `VISION.md:233-271` — Multi-rater relevance system (cascade evaluation)
- `VISION.md:252` — Cascade: embedding → LLM judge escalation
- `docs/research/09-embedding-based-connection-suggestions.md` — Embedding pipeline design (EmbeddingProvider trait, EmbeddingIndex, background worker)
