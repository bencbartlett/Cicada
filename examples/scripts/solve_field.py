# The wall's field-solver stage, as a Cicada Python script node (doc 10
# §5: scripts/ next to the pipeline self-register; the dialect calls them
# like stdlib nodes).
#
# Pure Python 3 on purpose — deterministic and dependency-free, so the
# example runs on any interpreter. The stage-6 corpus port brings the
# production numpy solver; the worker already marshals numpy arrays and
# scalars when scripts return them.

import cicada


@cicada.node(
    title="Field Solve",
    description="inverse-square field intensity at sample points.",
)
def solve_field(
    points: "[Point]",
    emitter: "Point" = (20.0, 12.0, 0.0),
    amps: "Number" = 400.0,
) -> "[Number]":
    ex, ey, ez = emitter
    out = []
    for (x, y, z) in points:
        d2 = (x - ex) ** 2 + (y - ey) ** 2 + (z - ez) ** 2
        out.append(amps / (1.0 + d2))
    return out
