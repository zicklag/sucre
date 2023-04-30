//! Contains the [`Edges`] container and iterators.

use std::sync::Arc;

use async_mutex::{Mutex, MutexGuardArc};
use crossbeam_channel::bounded;
use memmap2::MmapMut;
use rayon::{prelude::ParallelIterator, slice::ParallelSliceMut};

use super::*;

/// The size of an edge in bytes.
pub const EDGE_SIZE: usize = std::mem::size_of::<u64>() * U64S_PER_EDGE;

/// The number of [`u64`]s per edge storage.
///
/// Each edge is made up of 2 [`u64`]s.
pub const U64S_PER_EDGE: usize = 2;

/// Compressed adjacency matrix for storing node connections.
#[derive(Clone)]
pub struct Edges {
    mmap: Arc<Mutex<MmapMut>>,
}

impl Edges {
    /// Create a new, blank edges store.
    #[track_caller]
    pub fn new(memory_size: usize) -> Self {
        assert!(
            memory_size > EDGE_SIZE * 16,
            "edge memory size is too small"
        );
        Self {
            mmap: Arc::new(Mutex::new(
                MmapMut::map_anon(memory_size + (memory_size % EDGE_SIZE)).unwrap(),
            )),
        }
    }

    /// Allocate the edges in the given iterator in parallel.
    ///
    /// Returns `true` if the edge did not previously exist.
    #[track_caller]
    pub fn allocate<I: ExactSizeIterator<Item = Edge>>(&mut self, edges: I) {
        let mut mmap = self.mmap.try_lock().unwrap();

        // Fill a channel with all the edges we need to allocate
        let (sender, receiver) = bounded(edges.len());
        for edge in edges {
            sender.send(edge).unwrap();
        }

        // Cast our memory to from bytes to u64s to make it easier to extract our edge data from it.
        let u64s: &mut [u64] = bytemuck::cast_slice_mut(&mut mmap[..]);

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
                                let (id1, id2) = Self::ids_for_edge(edge_to_alloc);
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
        self.mmap.try_lock().unwrap().len() / EDGE_SIZE
    }

    /// Iterate over all of the pairs of nodes in the graph.
    #[track_caller]
    pub fn iter(&self) -> EdgeIter {
        EdgeIter::new(
            self.mmap
                .try_lock_arc()
                .expect("Can't iter: Edges is locked"),
        )
    }

    /// Iterate over all of the **active** pairs in the graph.
    ///
    /// An active pair is a pair of nodes connected by their `0` ports.
    pub fn active_pairs(&self) -> ActivePairIter {
        ActivePairIter(self.iter())
    }

    /// Helper to get the idx of an edge in the bitmap, given the nodes that it connects.
    #[track_caller]
    fn ids_for_edge(edge: Edge) -> (u64, u64) {
        let node_a = Self::id_for_node_port(edge.a, edge.a_port);
        let node_b = Self::id_for_node_port(edge.b, edge.b_port);
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
    fn id_for_node_port(mut node: NodeId, port: Uint) -> u64 {
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

/// Iterator over the pairs of connected nodes in an [`Edges`] store.
pub struct EdgeIter {
    mmap: MutexGuardArc<MmapMut>,
    edge_idx: usize,
}

impl EdgeIter {
    fn new(mmap: MutexGuardArc<MmapMut>) -> Self {
        Self { mmap, edge_idx: 0 }
    }
}

impl Iterator for EdgeIter {
    type Item = Edge;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.edge_idx * EDGE_SIZE >= self.mmap.len() {
                break None;
            }
            let idx = self.edge_idx;
            self.edge_idx += 1;
            let u64s: &[u64] = bytemuck::cast_slice(&self.mmap[..]);

            let id1 = u64s[idx * U64S_PER_EDGE];
            let id2 = u64s[idx * U64S_PER_EDGE + 1];

            let Some((a, a_port)) = Edges::node_port_from_id(id1) else {continue;};
            let (b, b_port) = Edges::node_port_from_id(id2).expect("Found only edge data");
            break Some(Edge {
                a,
                a_port,
                b,
                b_port,
            });
        }
    }
}

/// Iterator over the active pairs in an [`Edges`] store.
pub struct ActivePairIter(EdgeIter);

impl Iterator for ActivePairIter {
    type Item = (NodeId, NodeId);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .by_ref()
            .filter(|pair| pair.a_port == 0 && pair.b_port == 0)
            .map(|pair| (pair.a, pair.b))
            .next()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    /// Test [`Edges`] insertion and iteration.
    fn insert_and_iter() {
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
