# Sucre

_Experimental Symmetric Interaction Combinator Runtime_

This is my random experiment at developing a runtime for Symmetric Interaction Combinators.

The idea is to focus _only_ on efficiently evaluating interaction combinators, with the ability to call out to C/Rust/other programming languages during reduction. I don't want to deal with functional programming concerns, evaluating lambdas, etc. in the core. That should be done at a higher level library, though maybe I'll move that to like a `sucre_core` and then `sucre` will also include a functional language compiler, too.

This could be used as a lower-level layer to build, for example, an efficient functional programming language runtime, or something like [HVM].

Fair warning, I may not finish this, or even get anywhere with it. It's just a random side project.

[HVM]: https://github.com/HigherOrderCO/HVM
