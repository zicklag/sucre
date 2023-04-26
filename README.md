# Sucre

_Experimental Symmetric Interaction Combinator Runtime_

This is my random experiment at developing a runtime for Symmetric Interaction Combinators.

The idea is to focus _only_ on efficiently evaluating interaction combinators, with the ability to call out to C/Rust/other programming languages during reduction. I don't want to deal with functional programming concerns, evaluating lambdas, etc. in the core. That should be done at a higher level library, though maybe I'll move that to like a `sucre_core` and then `sucre` will also include a functional language compiler, too.

This could be used as a lower-level layer to build, for example, an efficient functional programming language runtime, or something like [HVM].

Fair warning, I may not finish this, or even get anywhere with it. It's just a random side project.

[HVM]: https://github.com/HigherOrderCO/HVM

## The Algorithm

> **Note:** Implementation is work-in-progress and has deviated a little from what's outlined here.
> If the implementation works I'll probably write an up-to-date blog post and video with an
> explanation of the algorithm.

The idea is to be very cache efficient and almost completely eliminate random memory accesses while reducing the graph.

This does not account for external function calls or data nodes, which, if this works, should be worked into the design.

- The graph is represented by:
  - A chunked list of nodes, with each chunk fitting into CPU cache.
    - The list of nodes is represented with two bits for each node.
      - 4 nodes can be stored in 1 `u8`.
      - Each set of 2 bits in the `u8` represents the node label, which is one of:
        - 0: null, the node is not present in this cell
        - 1: a constructor node
        - 2: a duplicator node
        - 3: an eraser node
  - A sparse adjacency matrix storing which nodes are connected to each-other.
    - The matrix is represented with a compressed ( roaring ) bitmap.
    - Only the bottom half of the matrix is stored in the bitmap.
      - Since undirected graphs have symmetric adjacency matrixes.
    - The matrix size is actually `3 × n` where `n` is the number of nodes.
    - Each set of three spots in the adjacency matrix represents the first, second, and third port of the node.
    - This means that the active port of every node is at an index divisible evenly by 3 in the adjacency matrix.

- **The algorithm progresses in phases:** each phase must complete before the next one can start.
  - **Phase 1:** search the adjacency matrix for active pairs in parallel.
    - The active pairs can be filtered out by searching the whole matrix for nodes that connect an index divisible by 3 to another index divisible by 3.
    - Send active pairs over a channel to be read in the next phase.
  - **Phase 2:** Collect and sort active pairs.
    - Collect the active pairs discoverd in phase 1, and sort them so that:
      - The lowest numbered node in the pair is always the first one in the pair, and
      - The pairs are sorted by their first node in the pair, with the lowest first.
    - Add an extra field to each pair in the list.
      - This field will store the node labels for both nodes in the active pair, but we don't know what the node labels are yet, so they will both start off empty.
      - This field might be implemented with two atomic bytes, one for each node.
  - **Phase 3:** Forward pass over nodes.
    - In parallel, iterate over each node chunk.
    - Each thread is assigned to it's share of nodes, that it will process in order.
    - As each thread walks through it's chunks, it walks through the list of active pairs, looking for the chunks that have one of the first nodes in an active pair in the chunk.
    - If it finds an active node in the chunk, it writes it's node type to the active pair list.
  - **Phase 4:** Backward pass over nodes.
    - **Note:** threads that finish with their chunks might be able to move on to phase 4 without waiting for the other threads still working on stage 3.
    - Once we get to the last chunk of nodes, we start going backward over the list of nodes.
    - This time we search for the chunks containing the second node in the active pairs list.
    - Once we find an active pair, we write it's node kind to the second node of the active pair.
    - Then we read the node kind of the other node that this node is connected to ( which was written in the forward pass ).
    - Now that we know what kind of node we are connected to, we can make either a duplication or annihilation.
      - **Annihilation**
        - we update the nodes in our chunk by deleting our active node
        - We also send the list of edge transformations for the annihilation over the edge transformation channel.
      - **Duplication**
        - We update our active node to one of the two nodes on our side of the graph that result from the duplication.
        - We also find an empty spot in our chunk to add the other node resulting for the duplication.
        - If we **don't** have an empty spot in our chunk, we send the info about the node we need to allocate over a channel to be allocated in the next chunk, if it has room.
        - Finally we push a list of edge changes over the edge change channel.
    - If we still have empty room in our chunk, we also check the pending allocation channel to see if we need to allocate any new nodes in our chunk.
      - If we allocate a node, we also need to push it's edge changes over the dge change channel.
    - If there are still nodes to allocate, and we don't have room in the last chunk, then start going back forward through the graph until we find an empty spot for the node.
  - **Phase 4:** Collect and apply edge transformations.
    - Collect all of the pending edge transformations from the edge change channel into a buffer.
    - Sort the buffer by the edge that is being removed.
    - Loop through the buffer, checking all the changes that were made to an individual edge, and combining those changes to determine the final edge.
    - This algorithm for combining might need a little bit of thought.

## What's the Name?

"Sucre" is French for "sugar". I was trying to come up with an acronym like Symmetric Interaction Combinator Runtime ( SICR ), but I didn't like SICR, since I couldn't figure out how to pronounce it. So I just went with Sucre, which was close, but doesn't have a good acronym and ends up just being a name. It looks like there's nothing major named Sucre on GitHub yet. 😃
