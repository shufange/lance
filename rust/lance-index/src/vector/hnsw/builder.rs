// Copyright 2024 Lance Developers.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Builder of Hnsw Graph.

use std::cmp::min;
use std::sync::Arc;

use lance_core::Result;
use log::{info, log_enabled, Level::Info};
use rand::{thread_rng, Rng};

use super::super::graph::{beam_search, memory::InMemoryVectorStorage};
use super::{select_neighbors, select_neighbors_heuristic, HNSW};
use crate::vector::graph::Graph;
use crate::vector::graph::{builder::GraphBuilder, storage::VectorStorage};
use crate::vector::hnsw::HnswLevel;

/// Build a HNSW graph.
///
/// Currently, the HNSW graph is fully built in memory.
///
/// During the build, the graph is built layer by layer.
///
/// Each node in the graph has a global ID which is the index on the base layer.
pub struct HNSWBuilder {
    /// max level of
    max_level: u16,

    /// max number of connections ifor each element per layers.
    m_max: usize,

    /// Size of the dynamic list for the candidates
    ef_construction: usize,

    /// Vector storage for the graph.
    vectors: Arc<InMemoryVectorStorage>,

    levels: Vec<GraphBuilder>,

    entry_point: u32,

    extend_candidates: bool,

    log_base: f32,

    use_select_heuristic: bool,
}

impl HNSWBuilder {
    /// Create a new [`HNSWBuilder`] with in memory vector storage.
    pub fn new(vectors: Arc<InMemoryVectorStorage>) -> Self {
        Self {
            max_level: 8,
            m_max: 64,
            ef_construction: 100,
            vectors,
            levels: vec![],
            entry_point: 0,
            extend_candidates: false,
            log_base: 10.0,
            use_select_heuristic: true,
        }
    }

    /// The maximum level of the graph.
    /// The default value is `8`.
    pub fn max_level(mut self, max_level: u16) -> Self {
        self.max_level = max_level;
        self
    }

    /// The maximum number of connections for each node per layer.
    /// The default value is `64`.
    pub fn max_num_edges(mut self, m_max: usize) -> Self {
        self.m_max = m_max;
        self
    }

    /// Number of candidates to be considered when searching for the nearest neighbors
    /// during the construction of the graph.
    ///
    /// The default value is `100`.
    pub fn ef_construction(mut self, ef_construction: usize) -> Self {
        self.ef_construction = ef_construction;
        self
    }

    /// Whether to expend to search candidate neighbors during heuristic search.
    ///
    /// The default value is `false`.
    ///
    /// See `extendCandidates` parameter in the paper (Algorithm 4)
    pub fn extend_candidates(mut self, flag: bool) -> Self {
        self.extend_candidates = flag;
        self
    }

    /// Use select heuristic when searching for the nearest neighbors.
    ///
    /// See algorithm 4 in HNSW paper.
    pub fn use_select_heuristic(mut self, flag: bool) -> Self {
        self.use_select_heuristic = flag;
        self
    }

    #[inline]
    fn len(&self) -> usize {
        self.levels[0].len()
    }

    /// New node's level
    ///
    /// See paper `Algorithm 1`
    fn random_level(&self) -> u16 {
        let mut rng = thread_rng();
        // This is different to the paper.
        // We use log10 instead of log(e), so each layer has about 1/10 of its bottom layer.
        let m = self.vectors.len();
        min(
            (m as f32).log(self.log_base).ceil() as u16
                - (rng.gen::<f32>() * self.vectors.len() as f32)
                    .log(self.log_base)
                    .ceil() as u16,
            self.max_level,
        )
    }

    /// Insert one node.
    fn insert(&mut self, node: u32) -> Result<()> {
        let vector = self.vectors.vector(node);
        let level = self.random_level();

        let levels_to_search = if self.levels.len() > level as usize {
            self.levels.len() - level as usize - 1
        } else {
            0
        };
        let mut ep = vec![self.entry_point];
        let query = self.vectors.vector(self.entry_point);

        //
        // Search for entry point in paper.
        // ```
        //   for l_c in (L..l+1) {
        //     W = Search-Layer(q, ep, ef=1, l_c)
        //    ep = Select-Neighbors(W, 1)
        //  }
        // ```
        for cur_level in self.levels.iter().rev().take(levels_to_search) {
            let candidates = beam_search(cur_level, &ep, vector, 1)?;
            ep = select_neighbors(&candidates, 1).map(|(_, id)| id).collect()
        }

        let m = self.len();
        for cur_level in self.levels.iter_mut().rev().skip(levels_to_search) {
            cur_level.insert(node);
            let candidates = beam_search(cur_level, &ep, vector, self.ef_construction)?;
            let neighbours: Vec<_> = if self.use_select_heuristic {
                select_neighbors_heuristic(cur_level, query, &candidates, m, self.extend_candidates)
                    .collect()
            } else {
                select_neighbors(&candidates, m).collect()
            };

            for (_, nb) in neighbours.iter() {
                cur_level.connect(node, *nb)?;
            }
            for (_, nb) in neighbours {
                cur_level.prune(nb, self.m_max)?;
            }
            cur_level.prune(node, self.m_max)?;
            ep = candidates.values().copied().collect();
        }

        if level > self.levels.len() as u16 {
            self.entry_point = node;
        }

        Ok(())
    }

    /// Build the graph, with the already provided `VectorStorage` as backing storage for HNSW graph.
    pub fn build(&mut self) -> Result<HNSW> {
        self.build_with(self.vectors.clone())
    }

    /// Build the graph, with the provided [VectorStorage] as backing storage for HNSW graph.
    pub fn build_with(&mut self, storage: Arc<dyn VectorStorage>) -> Result<HNSW> {
        log::info!(
            "Building HNSW graph: metric_type={}, max_levels={}, m_max={}, ef_construction={}",
            self.vectors.metric_type(),
            self.max_level,
            self.m_max,
            self.ef_construction
        );
        for _ in 0..self.max_level {
            let mut level = GraphBuilder::new(self.vectors.clone());
            level.insert(0);
            self.levels.push(level);
        }

        for i in 1..self.vectors.len() {
            self.insert(i as u32)?;
        }

        if log_enabled!(Info) {
            for (i, level) in self.levels.iter().enumerate() {
                info!("HNSW level {}: {:#?}", i, level.stats());
            }
        }

        let graphs = self
            .levels
            .iter()
            .map(|l| HnswLevel::from_builder(l, storage.clone()))
            .collect::<Result<Vec<_>>>()?;
        Ok(HNSW::from_builder(
            graphs,
            self.entry_point,
            self.vectors.metric_type(),
            self.use_select_heuristic,
        ))
    }
}
