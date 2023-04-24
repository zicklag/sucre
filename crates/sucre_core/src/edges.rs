//! Contains the [`Edges`] container and iterators.

use std::collections::HashMap;

use super::*;

/// Compressed adjacency matrix for storing node connections.
#[derive(Default)]
pub struct Edges {
    /// The bitmap that stores the adjacency matrix.
    bitset: RoaringTreemap,
    /// Cache of edges that have been moved during a call to [`Edges::apply_mutations`].
    dissolved_edges: HashMap<Edge, Edge>,
}

impl Edges {
    /// Create a new, blank edges store.
    pub fn new() -> Self {
        Default::default()
    }

    /// Get whether or not the given nodes are connected to each-other
    pub fn get(&self, edge: Edge) -> bool {
        let idx = Self::idx_for_edge(edge);
        self.bitset.contains(idx)
    }

    /// Insert an edge between the two given nodes.
    ///
    /// Returns `true` if the edge did not previously exist.
    #[track_caller]
    pub fn insert(&mut self, edge: Edge) -> bool {
        let idx = Self::idx_for_edge(edge);
        self.bitset.insert(idx)
    }

    /// Remove an edge between two nodes.
    ///
    /// Returns `true` if the edge previously existed.
    pub fn remove(&mut self, edge: Edge) -> bool {
        let idx = Self::idx_for_edge(edge);
        self.bitset.remove(idx)
    }

    /// Count the edges in
    pub fn count(&self) -> Uint {
        self.bitset.len()
    }

    /// Iterate over all of the pairs of nodes in the graph.
    pub fn iter(&self) -> EdgeIter {
        EdgeIter::new(self.bitset.iter())
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
                EdgeMutation::Dissolve { mut a, b, mut c } => {
                    // If the either of the outside edges have been dissovled, operate on the new
                    // edges that replaced the dissolved one.
                    while let Some(replaced_edge) = self.dissolved_edges.get(&a) {
                        a = *replaced_edge;
                    }
                    while let Some(replaced_edge) = self.dissolved_edges.get(&c) {
                        c = *replaced_edge;
                    }

                    // Find the outside edge of a and the inside edge of c
                    let (a_outside, a_outside_port, c_inside) = if a.a == b.a {
                        (a.b, a.b_port, b.b)
                    } else if a.b == b.a {
                        (a.a, a.a_port, b.b)
                    } else if a.a == b.b {
                        (a.b, a.b_port, b.a)
                    } else if a.b == b.b {
                        (a.a, a.a_port, b.a)
                    } else {
                        unreachable!("Invalid dissolve");
                    };

                    // Find the outside edge of c
                    let (c_outside, c_outside_port) = if c_inside == c.a {
                        (c.b, c.b_port)
                    } else if c_inside == c.b {
                        (c.a, c.a_port)
                    } else {
                        unreachable!("Invalid dissolve");
                    };

                    // Remove the individual edges
                    self.remove(a);
                    self.remove(b);
                    self.remove(c);

                    // Create a new edge connecting the ends of a and c
                    let new_edge = Edge {
                        a: a_outside,
                        b: c_outside,
                        a_port: a_outside_port,
                        b_port: c_outside_port,
                    }
                    .canonical();
                    self.insert(new_edge);

                    // Record the edges a and c as having been dissolved
                    self.dissolved_edges.insert(a, new_edge);
                    self.dissolved_edges.insert(c, new_edge);
                }
                EdgeMutation::Insert(edge) => {
                    self.insert(edge);
                }
                EdgeMutation::Remove(edge) => {
                    self.remove(edge);
                }
            }
        }
    }

    /// Helper to get the idx of an edge in the bitmap, given the nodes that it connects.
    #[track_caller]
    fn idx_for_edge(edge: Edge) -> Uint {
        debug_assert!(
            edge.a_port < 3,
            "Node a's port id must be between 0 and 2 inclusive."
        );
        debug_assert!(
            edge.b_port < 3,
            "Node b's port id must be between 0 and 2 inclusive."
        );
        let a = edge.a * 3 + edge.a_port;
        let b = edge.b * 3 + edge.b_port;
        let x = a.min(b);
        let y = a.max(b);
        recursive_sum(y) + x
    }
}

/// A mutation that may be made to an edge in [`Edges`].
#[derive(Debug, Clone, Copy)]
pub enum EdgeMutation {
    /// Dissolve the three given edges, into one edge connecting the outside of `a` with the outside
    /// of `c`, and deleting the center edge `b`.
    Dissolve {
        /// The first edge to connect.
        a: Edge,
        /// The second edge to connect.
        b: Edge,
        /// The third edge to connect.
        c: Edge,
    },
    /// Insert a new edge.
    Insert(Edge),
    /// Remove an existing edge.
    Remove(Edge),
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
    iter: roaring::treemap::Iter<'a>,
    y: Uint,
    y_sum: Uint,
    next_y_sum: Uint,
}

impl<'a> EdgeIter<'a> {
    fn new(iter: roaring::treemap::Iter<'a>) -> Self {
        Self {
            iter,
            y: 0,
            y_sum: 0,
            next_y_sum: 1,
        }
    }
}

impl<'a> Iterator for EdgeIter<'a> {
    type Item = Edge;

    fn next(&mut self) -> Option<Self::Item> {
        let edge_idx = self.iter.next()?;

        // TODO(perf): find a faster way to do this maybe?
        while self.next_y_sum <= edge_idx {
            self.y_sum = self.next_y_sum;
            self.y += 1;
            self.next_y_sum = self.y_sum + self.y + 1;
        }
        let x = edge_idx - self.y_sum;

        Some(
            Edge {
                a: x / 3,
                b: self.y / 3,
                a_port: x % 3,
                b_port: self.y % 3,
            }
            // Make sure the pair has the lowest-index node in `a`.
            .canonical(),
        )
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

/// This helper returns the equivalent of a factorial for addition.
// TODO(docs): There's a name for this, but I can't remember what it is. Update the name to use the
// mathematical name for the construct.
// TODO(perf): Benchmark with and without the memoization.
#[cached::proc_macro::cached]
fn recursive_sum(n: Uint) -> Uint {
    if n == 0 {
        0
    } else {
        n + recursive_sum(n - 1)
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

    /// Test that we can apply multiple annihilations on a couple nodes that are connected to
    /// each-other and produce the correct result.
    #[test]
    fn test_apply_multiple_dissolve_mutations() {
        let mut edges = Edges::new();

        // Create some nodes
        //                                                                      a   b  c  d  e    f   g     h
        let (a, b, c, d, e, f, g, h) = (10, 5, 8, 7, 104, 80, 1035, 195);

        // Connect them together to form a structure that will have two annihilations manipulating
        // the same nodes.
        let e = [
            (a, 0, c, 1),
            (b, 0, c, 2),
            (c, 0, d, 0),
            (d, 2, e, 1),
            (d, 1, e, 2),
            (e, 0, f, 0),
            (f, 2, g, 0),
            (f, 1, h, 0),
        ]
        .into_iter()
        .map(Edge::from)
        .collect::<Vec<_>>();

        for edge in &e {
            edges.insert(*edge);
        }

        // Manually create the edge dissolves that would be computed during graph reduction
        let mutations = [
            EdgeMutation::Dissolve {
                a: e[0],
                b: e[2],
                c: e[4],
            },
            EdgeMutation::Dissolve {
                a: e[1],
                b: e[2],
                c: e[3],
            },
            EdgeMutation::Dissolve {
                a: e[3],
                b: e[5],
                c: e[7],
            },
            EdgeMutation::Dissolve {
                a: e[4],
                b: e[5],
                c: e[6],
            },
        ];

        // Apply the mutations
        edges.apply_mutations(mutations);

        // Check that we now have only two edges
        assert_eq!(edges.count(), 2);

        // Make sure we have both of the edges that we expect
        let expected = [Edge::from((a, 0, g, 0)), Edge::from((b, 0, h, 0))];

        for edge in edges.iter() {
            assert!(
                expected.iter().any(|x| x == &edge),
                "Couldn't find expected edge"
            )
        }
    }
}
