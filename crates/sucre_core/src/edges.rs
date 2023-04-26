//! Contains the [`Edges`] container and iterators.

use std::collections::HashMap;

use super::*;

/// Compressed adjacency matrix for storing node connections.
#[derive(Clone, Default)]
pub struct Edges {
    /// Hash map that stores edges by mapping a node and port to the node and port that it's
    /// connected to.
    ///
    /// The first 62 bits of the u64 are used to store the node ID, and the last two bits store the
    /// port number.
    map: HashMap<u64, u64>,
    /// Cache of edges that have been moved during a call to [`Edges::apply_mutations`].
    dissolved_edges: HashMap<Edge, Edge>,
}

impl Edges {
    /// Create a new, blank edges store.
    pub fn new() -> Self {
        Default::default()
    }

    /// Get whether or not the given nodes are connected to each-other
    #[track_caller]
    pub fn get(&self, mut node: u64, port: Uint) -> Option<(u64, Uint)> {
        assert_eq!(
            node.get_bits(62..64),
            0,
            "Node ID too high to store in `Edges`"
        );
        node.set_bits(63..64, port);
        self.map.get(&node).map(|node| {
            let mut node = *node;
            let port = node.get_bits(62..64);
            node.set_bits(62..64, 0);
            (node, port)
        })
    }

    /// Insert an edge between the two given nodes.
    ///
    /// Returns `true` if the edge did not previously exist.
    #[track_caller]
    pub fn insert(&mut self, edge: Edge) -> bool {
        let (a, b) = Self::ids_for_edge(edge);
        self.map.insert(a, b).is_none() && self.map.insert(b, a).is_none()
    }

    /// Remove an edge between two nodes.
    ///
    /// Returns `true` if the edge previously existed.
    pub fn remove(&mut self, edge: Edge) -> bool {
        let (a, b) = Self::ids_for_edge(edge);
        self.map.remove(&a).is_some() && self.map.remove(&b).is_some()
    }

    /// Count the edges.
    pub fn count(&self) -> usize {
        // Divide length by two because we have two entries for each edge.
        self.map.len() / 2
    }

    /// Iterate over all of the pairs of nodes in the graph.
    pub fn iter(&self) -> EdgeIter {
        EdgeIter::new(self.map.iter())
    }

    /// Iterate over all of the **active** pairs in the graph.
    ///
    /// An active pair is a pair of nodes connected by their `0` ports.
    pub fn active_pairs(&self) -> ActivePairIter {
        ActivePairIter(self.iter())
    }

    /// Apply an iterator of mutations to the edge store.
    pub fn apply_mutations<I>(&mut self, mutations: I)
    where
        I: IntoIterator<Item = EdgeMutation>,
    {
        self.dissolved_edges.clear();
        let mutations = mutations.into_iter();

        for mutation in mutations {
            match mutation {
                EdgeMutation::Reconnect(Edge {
                    a,
                    a_port,
                    b,
                    b_port,
                }) => {
                    // Find out what node b is connected to.
                    let (b2, b2_port) = self.get(b, b_port).expect("missing edge");

                    // Remove the connection between `b` and `b2`
                    self.remove(Edge {
                        a: b2,
                        b,
                        a_port: b2_port,
                        b_port,
                    });

                    // Find out what node a is connected to.
                    let (a2, a2_port) = self.get(a, a_port).expect("missing edge");

                    // Remove the connection between `a` and `a2`
                    self.remove(Edge {
                        a,
                        b: a2,
                        a_port,
                        b_port: a2_port,
                    });

                    // Connect `a` to `b2`
                    self.insert(Edge {
                        a,
                        b: b2,
                        a_port,
                        b_port: b2_port,
                    });
                }
                EdgeMutation::Bridge(Edge {
                    a,
                    a_port,
                    b,
                    b_port,
                }) => {
                    // Find out what node b is connected to.
                    let (b2, b2_port) = self.get(b, b_port).expect("missing edge");

                    // Remove the connection between `b` and `b2`
                    self.remove(Edge {
                        a: b2,
                        b,
                        a_port: b2_port,
                        b_port,
                    });

                    // Find out what node a is connected to.
                    let (a2, a2_port) = self.get(a, a_port).expect("missing edge");

                    // Remove the connection between `a` and `a2`
                    self.remove(Edge {
                        a,
                        b: a2,
                        a_port,
                        b_port: a2_port,
                    });

                    // Connect `a2` to `b2`
                    self.insert(Edge {
                        a: a2,
                        b: b2,
                        a_port: a2_port,
                        b_port: b2_port,
                    });
                }
                EdgeMutation::InsertEdge(edge) => {
                    self.insert(edge);
                }
                EdgeMutation::RemoveEdge(edge) => {
                    self.remove(edge);
                }
            }
        }
    }

    /// Helper to get the idx of an edge in the bitmap, given the nodes that it connects.
    #[track_caller]
    fn ids_for_edge(edge: Edge) -> (u64, u64) {
        let mut node_a = edge.a;
        let mut node_b = edge.b;
        assert!(edge.a_port < 3, "Port `a` is too high.");
        assert!(edge.b_port < 3, "Port `a` is too high.");
        assert_eq!(
            node_a.get_bits(62..64),
            0,
            "Node ID `a` too high to store in `Edges`"
        );
        assert_eq!(
            node_a.get_bits(62..64),
            0,
            "Node ID `b` too high to store in `Edges`"
        );

        node_a.set_bits(62..64, edge.a_port);
        node_b.set_bits(62..64, edge.b_port);

        (node_a, node_b)
    }

    /// Helper to get the node and port from an edge ID.
    fn node_port_from_id(mut id: u64) -> (NodeId, Uint) {
        let port = id.get_bits(62..64);
        id.set_bits(62..64, 0);
        (id, port)
    }
}

/// A mutation that may be made to an edge in [`Edges`].
#[derive(Debug, Clone, Copy)]
pub enum EdgeMutation {
    /// Connect node `a`'s `a_port` to whatever node `b`s `b_port` was connected to.
    Reconnect(Edge),
    /// Connect whatever was connected to `a`s `a_port` to whatever was connected to `b`s `b_port`.
    ///
    /// This is similar to [`Reconnect`], but subtly different, because it doesn't connect a to b,
    /// it connects _whatever is connect to a_, to b.
    Bridge(Edge),
    /// Insert a new edge.
    InsertEdge(Edge),
    /// Remove an existing edge.
    RemoveEdge(Edge),
}

// TODO(perf): We know that the `a_port` and `b_port` are going to be between 1 and 3, so should we
// really use a whole [`Uint`] to store them?
/// A pair of connected nodes, returned by [`Edges::pairs()`].
#[derive(Eq, PartialEq, Clone, Copy, Debug, Hash)]
pub struct Edge {
    /// The first connected node.
    pub a: NodeId,
    /// The second connected node.
    pub b: NodeId,
    /// The port that the first node is connected by.
    pub a_port: Uint,
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
pub struct EdgeIter<'a> {
    iter: std::collections::hash_map::Iter<'a, u64, u64>,
}

impl<'a> EdgeIter<'a> {
    fn new(iter: std::collections::hash_map::Iter<'a, u64, u64>) -> Self {
        Self { iter }
    }
}

impl<'a> Iterator for EdgeIter<'a> {
    type Item = Edge;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (a, b) = self.iter.next()?;
            let (a, a_port) = Edges::node_port_from_id(*a);
            let (b, b_port) = Edges::node_port_from_id(*b);

            // Since the map contains two entries for each edge, we make sure we return each edge
            // once by only returning edges where `a` is less than `b` or where `a_port` is less
            // than `b_port` when the node is connected to itself.
            if a > b || (a == b && a_port > b_port) {
                continue;
            }

            return Some(Edge {
                a,
                b,
                a_port,
                b_port,
            });
        }
    }
}

/// Iterator over the active pairs in an [`Edges`] store.
pub struct ActivePairIter<'a>(EdgeIter<'a>);

impl<'a> Iterator for ActivePairIter<'a> {
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
        let mut edges = Edges::new();

        // Create some nodes ( use random numbers since the nodes won't always be in order in real
        // operation )
        //                                                      a  b   c   d   e  f
        let (a, b, c, d, e, f) = (0, 20, 53, 13, 2, 33);

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
        for (a, ap, b, bp) in edges_to_create {
            edges.insert(Edge {
                a,
                b,
                a_port: ap,
                b_port: bp,
            });
        }

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
        assert_eq!(active_pairs, vec![(d, b)]);
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
