//! Contains the [`Edges`] container and iterators.

use crossbeam_channel::{bounded, Receiver};
use memmap2::MmapMut;
use rayon::{
    prelude::ParallelIterator,
    slice::{ParallelSlice, ParallelSliceMut},
};

use super::*;

/// The size of an edge in bytes.
pub const EDGE_SIZE: usize = std::mem::size_of::<u64>() * U64S_PER_EDGE;

/// The number of [`u64`]s per edge storage.
///
/// Each edge is made up of 2 [`u64`]s.
pub const U64S_PER_EDGE: usize = 2;

/// Compressed adjacency matrix for storing node connections.
pub struct Edges {
    mmap: MmapMut,
    /// Buffer re-used in calls to [`Self::apply_mutations()`].
    pending_mutations: Vec<EdgeMutation>,
}

impl Clone for Edges {
    fn clone(&self) -> Self {
        let mut mmap = MmapMut::map_anon(self.mmap.len()).unwrap();
        mmap.advise(memmap2::Advice::Sequential).ok();
        mmap.copy_from_slice(&self.mmap);
        Self {
            mmap,
            pending_mutations: Vec::new(),
        }
    }
}

impl Edges {
    /// Create a new, blank edges store.
    #[track_caller]
    pub fn new(memory_size: usize) -> Self {
        assert!(
            memory_size > EDGE_SIZE * 16,
            "edge memory size is too small"
        );
        let mmap = MmapMut::map_anon(memory_size + (memory_size % EDGE_SIZE)).unwrap();
        mmap.advise(memmap2::Advice::Sequential).ok();
        Self {
            mmap,
            pending_mutations: Vec::new(),
        }
    }

    /// Allocate the edges in the given iterator in parallel.
    ///
    /// Returns `true` if the edge did not previously exist.
    #[track_caller]
    pub fn allocate<I: ExactSizeIterator<Item = Edge>>(&mut self, edges: I) {
        // Fill a channel with all the edges we need to allocate
        let (sender, receiver) = bounded(edges.len());
        for edge in edges {
            sender.send(edge).unwrap();
        }

        // Cast our memory to from bytes to u64s to make it easier to extract our edge data from it.
        let u64s: &mut [u64] = bytemuck::cast_slice_mut(&mut self.mmap[..]);

        // While there are still edges to allocate
        while !receiver.is_empty() {
            // Iterate over the memory in parallel
            // TODO(perf): evaluate whether it's more efficient to iterate in chunks like this,
            // or whether it's better to to iterate over every edge slot.
            u64s.par_chunks_mut(CHUNK_SIZE / EDGE_SIZE)
                .for_each(|chunk| {
                    // Split the chunk into edges
                    let mut edges = chunk.chunks_mut(U64S_PER_EDGE);

                    // For every edge we need to allocate
                    'edges: for edge_to_alloc in receiver.try_iter() {
                        loop {
                            // Get the next edge in our chunk
                            let Some(edge) = edges.next() else {
                            // If we have run out of spots in our chunk,
                            // send the edge back into the allocation queue,
                            // because we don't have a spot for it.
                            sender.send(edge_to_alloc).unwrap();
                            return;
                        };

                            // If this slot doesn't contain an edge
                            if Self::node_port_from_id(edge[0]).is_none() {
                                debug_assert!(
                                    Self::node_port_from_id(edge[1]).is_none(),
                                    "Found a half-allocated edge."
                                );

                                // Allocate the edge here
                                let (id1, id2) = Self::ids_from_edge(edge_to_alloc);
                                edge[0] = id1;
                                edge[1] = id2;

                                // Try to allocate the next edge
                                continue 'edges;

                            // There's no open spot here
                            } else {
                                // Check the next spot
                                continue;
                            }
                        }
                    }
                });
        }
    }

    /// Count the edges.
    pub fn count(&self) -> usize {
        self.mmap.len() / EDGE_SIZE
    }

    /// Iterate over all of the pairs of nodes in the graph.
    pub fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        self.as_u64s()
            .chunks_exact(U64S_PER_EDGE)
            .filter_map(Self::edge_for_ids)
    }

    /// Iterate over all of the **active** pairs of nodes in the graph.
    pub fn active_pairs(&self) -> impl Iterator<Item = (NodeId, NodeId)> + '_ {
        self.iter()
            .filter(|e| e.a_port == 0 && e.b_port == 0)
            .map(|e| (e.a, e.b))
    }

    /// Iterate in parallel over all the pairs of nodes in the graph.
    pub fn par_iter(&self) -> impl ParallelIterator<Item = Edge> + '_ {
        self.as_u64s()
            .par_chunks_exact(U64S_PER_EDGE)
            .filter_map(Self::edge_for_ids)
    }

    /// Iterate in parallel over all of the **active** pairs of nodes in the graph.
    pub fn par_active_pairs(&self) -> impl ParallelIterator<Item = (NodeId, NodeId)> + '_ {
        self.par_iter()
            .filter(|e| e.a_port == 0 && e.b_port == 0)
            .map(|e| (e.a, e.b))
    }

    /// Apply edge mutations.
    pub fn apply_mutations<I: IntoIterator<Item = EdgeMutation>>(&mut self, mutations: I) {
        let Self {
            mmap,
            pending_mutations,
        } = self;
        let u64s: &mut [u64] = bytemuck::cast_slice_mut(&mut mmap[..]);

        // Collect the pending mutations into our buffer
        let mutations = mutations.into_iter();
        pending_mutations.clear();
        pending_mutations.extend(mutations);

        u64s.par_chunks_mut(CHUNK_SIZE * U64S_PER_EDGE)
            .for_each(|chunk| {
                'mutations: for mutation in &self.pending_mutations {
                    match mutation {
                        EdgeMutation::Annihilate { a, b } => {
                            let active_edge = Edge {
                                a: *a,
                                b: *b,
                                a_port: 0,
                                b_port: 0,
                            }
                            .canonical();

                            for ids in chunk.chunks_exact_mut(U64S_PER_EDGE) {
                                let Some(edge) = Self::edge_for_ids(ids) else { continue; };

                                // Delete the active edge
                                if edge.canonical() == active_edge {
                                    ids[0] = 0;
                                    ids[1] = 0;
                                    continue;
                                }

                                for id in ids {
                                    let (node, port) = Self::node_port_from_id(*id).unwrap();

                                    if node == *a && port == 1 {
                                        *id = Self::id_from_node_port(*b, 2);
                                    } else if node == *b && port == 2 {
                                        // Connect it to the eraser's port 0
                                        *id = Self::id_from_node_port(*a, 1)
                                    }
                                }
                            }
                        }
                        EdgeMutation::EraseEdge(edge_to_delete) => {
                            let edge_to_delete = edge_to_delete.canonical();
                            // For all of the edges in the chunk
                            for ids in chunk.chunks_exact_mut(U64S_PER_EDGE) {
                                let Some(edge) = Self::edge_for_ids(ids) else { continue; };

                                // If this edge matches the one to delete
                                if edge.canonical() == edge_to_delete {
                                    // Delete it
                                    ids[0] = 0;
                                    ids[1] = 0;
                                    continue 'mutations;
                                }
                            }
                        }
                        EdgeMutation::EraseConstructor {
                            constructor,
                            eraser,
                        } => {
                            let active_edge = Edge {
                                a: *constructor,
                                b: *eraser,
                                a_port: 0,
                                b_port: 0,
                            }
                            .canonical();

                            // For all of the edges in the chunk
                            for ids in chunk.chunks_exact_mut(U64S_PER_EDGE) {
                                let Some(edge) = Self::edge_for_ids(ids) else { continue; };

                                // If this is the active edge
                                if edge.canonical() == active_edge {
                                    // Delete it
                                    ids[0] = 0;
                                    ids[1] = 0;
                                    continue;
                                }

                                for id in ids {
                                    let (node, port) = Self::node_port_from_id(*id).unwrap();

                                    // If this is constructor port 1
                                    if node == *constructor && port == 1 {
                                        // Connect it to constructor port 0 ( which constructor will
                                        // actually be turned into an eraser by now )
                                        *id = Self::id_from_node_port(*constructor, 0);

                                    // If this is the constructor port 2
                                    } else if node == *constructor && port == 2 {
                                        // Connect it to the eraser's port 0
                                        *id = Self::id_from_node_port(*eraser, 0)
                                    }
                                }
                            }
                        }
                    }
                }
            });
    }
}

/// Internal helper functions
impl Edges {
    // Helper to borrow the internal mmap as a slice of u64s
    fn as_u64s(&self) -> &[u64] {
        bytemuck::cast_slice(&self.mmap[..])
    }

    /// Helper to get the edge stored in the given two u64s.
    fn edge_for_ids(ids: &[u64]) -> Option<Edge> {
        debug_assert_eq!(ids.len(), 2);
        Self::node_port_from_id(ids[0]).map(|(a, a_port)| {
            let (b, b_port) = Self::node_port_from_id(ids[1]).expect("Half edge");

            Edge {
                a,
                a_port,
                b,
                b_port,
            }
        })
    }

    /// Helper to get the idx of an edge in the bitmap, given the nodes that it connects.
    #[track_caller]
    fn ids_from_edge(edge: Edge) -> (u64, u64) {
        let node_a = Self::id_from_node_port(edge.a, edge.a_port);
        let node_b = Self::id_from_node_port(edge.b, edge.b_port);
        (node_a, node_b)
    }

    /// Helper to get the u64 id for a node⋄port pair.
    ///
    /// The pair is encoded with the first 62 bits used for the node ID and the last 2 bits used to
    /// represent the port.
    ///
    /// The ID could also represent a null node, in which case the port will be 0.
    ///
    /// Since port 0 is a valid port number, we encode a port of zero as 1.
    #[track_caller]
    fn id_from_node_port(mut node: NodeId, port: Uint) -> u64 {
        assert!(port < 3, "Port is too high.");
        assert_eq!(
            node.get_bits(62..64),
            0,
            "Node ID too high to store in `Edges`"
        );
        node.set_bits(62..64, port + 1);
        node
    }

    /// Helper to get the node and port from an edge ID.
    fn node_port_from_id(mut id: u64) -> Option<(NodeId, Uint)> {
        let port = id.get_bits(62..64);
        id.set_bits(62..64, 0);
        if port == 0 {
            None
        } else {
            Some((id, port - 1))
        }
    }
}

/// Queued mutation that will be applied to [`Edges`].
pub enum EdgeMutation {
    EraseEdge(Edge),
    EraseConstructor { constructor: NodeId, eraser: NodeId },
    Annihilate { a: NodeId, b: NodeId },
}

// TODO(perf): We know that the `a_port` and `b_port` are going to be between 1 and 3, so should we
// really use a whole [`Uint`] to store them?
/// A pair of connected nodes, returned by [`Edges::pairs()`].
#[derive(Eq, PartialEq, Clone, Copy, Debug, Hash)]
pub struct Edge {
    /// The first connected node.
    pub a: NodeId,
    /// The port that the first node is connected by.
    pub a_port: Uint,
    /// The second connected node.
    pub b: NodeId,
    /// The port that the second node is connected by.
    pub b_port: Uint,
}

impl Edge {
    // Return the "canonical" version of the pair, which means that the [`NodeId`] for `a` will
    /// always be less than the [`NodeId`] for `b`.
    pub fn canonical(self) -> Self {
        if self.a <= self.b {
            self
        } else {
            Edge {
                a: self.b,
                b: self.a,
                a_port: self.b_port,
                b_port: self.a_port,
            }
        }
    }
}

impl From<(NodeId, Uint, NodeId, Uint)> for Edge {
    fn from((a, ap, b, bp): (NodeId, Uint, NodeId, Uint)) -> Self {
        Self {
            a,
            b,
            a_port: ap,
            b_port: bp,
        }
        .canonical()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    /// Test [`Edges`] insertion and iteration.
    fn allocate_and_iter() {
        let mut edges = Edges::new(1024);

        // Create some nodes ( use random numbers since the nodes won't always be in order in real
        // operation )
        //                                                      a  b   c   d   e  f
        let (a, b, c, d, e, f) = (0, 13, 53, 20, 2, 33);

        // Connect them in the form of the interaction net for `(λx.xx)(λx.x)`
        let edges_to_create = [
            (a, 0, b, 2),
            (b, 0, d, 0), // Active pair
            (b, 1, c, 0),
            (c, 1, c, 2),
            (d, 1, f, 0),
            (d, 2, e, 2),
            (f, 2, e, 0),
            (f, 1, e, 1),
        ];

        // Insert edges from list
        edges.allocate(edges_to_create.iter().copied().map(|(a, ap, b, bp)| Edge {
            a,
            b,
            a_port: ap,
            b_port: bp,
        }));

        // Make sure the iterator returns the proper number of pairs
        let pairs = edges.iter();

        // Make sure the iterator returns all of our pairs
        let mut edges_to_find = edges_to_create
            .iter()
            .map(|(a, ap, b, bp)| Edge {
                a: *a,
                b: *b,
                a_port: *ap,
                b_port: *bp,
            })
            .collect::<Vec<_>>();
        for pair in pairs {
            edges_to_find.remove(
                edges_to_find
                    .iter()
                    .position(|x| x.canonical() == pair.canonical())
                    .unwrap_or_else(|| panic!("Pair not found: {pair:?}")),
            );
        }
        assert!(edges_to_find.is_empty());

        // Make sure the active pair iterator returns our only active pair
        let active_pairs = edges.active_pairs().collect::<Vec<_>>();
        assert_eq!(active_pairs, vec![(b, d)]);
    }

    // /// Test that we can apply multiple annihilations on a couple nodes that are connected to
    // /// each-other and produce the correct result.
    // #[test]
    // fn test_apply_multiple_dissolve_mutations() {
    //     let mut edges = Edges::new();

    //     // Create some nodes
    //     //                                                                      a   b  c  d  e    f   g     h
    //     let (a, b, c, d, e, f, g, h) = (10, 5, 8, 7, 104, 80, 1035, 195);

    //     // Connect them together to form a structure that will have two annihilations manipulating
    //     // the same nodes.
    //     let e = [
    //         (a, 0, c, 1),
    //         (b, 0, c, 2),
    //         (c, 0, d, 0),
    //         (d, 2, e, 1),
    //         (d, 1, e, 2),
    //         (e, 0, f, 0),
    //         (f, 2, g, 0),
    //         (f, 1, h, 0),
    //     ]
    //     .into_iter()
    //     .map(Edge::from)
    //     .collect::<Vec<_>>();

    //     for edge in &e {
    //         edges.insert(*edge);
    //     }

    //     // Manually create the edge dissolves that would be computed during graph reduction
    //     let mutations = [
    //         EdgeMutation::Dissolve {
    //             a: e[0],
    //             b: e[2],
    //             c: e[4],
    //         },
    //         EdgeMutation::Dissolve {
    //             a: e[1],
    //             b: e[2],
    //             c: e[3],
    //         },
    //         EdgeMutation::Dissolve {
    //             a: e[3],
    //             b: e[5],
    //             c: e[7],
    //         },
    //         EdgeMutation::Dissolve {
    //             a: e[4],
    //             b: e[5],
    //             c: e[6],
    //         },
    //     ];

    //     // Apply the mutations
    //     edges.apply_mutations(mutations);

    //     // Check that we now have only two edges
    //     assert_eq!(edges.count(), 2);

    //     // Make sure we have both of the edges that we expect
    //     let expected = [Edge::from((a, 0, g, 0)), Edge::from((b, 0, h, 0))];

    //     for edge in edges.iter() {
    //         assert!(
    //             expected.iter().any(|x| x == &edge),
    //             "Couldn't find expected edge"
    //         )
    //     }
    // }
}
