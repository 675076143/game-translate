# Engineering Principles

1. Do not preserve backward compatibility. Delete obsolete implementations; do not add compatibility layers, migrations, or fallbacks.
2. Choose the simplest implementation that meets current requirements. Avoid speculative abstractions and unnecessary configuration layers.
3. Build vertical slices. Make the smallest end-to-end version work before adding complexity, and never dismantle a working slice for unfinished complexity.
4. Keep components modular and concerns separated.
5. Prefer mature, maintained libraries. Do not rewrite existing solutions without a concrete reason.
6. Inspect existing dependencies before adding a package or implementing functionality ourselves.
7. Make architecture decisions for the long term. Do not knowingly add temporary designs that must be replaced later.
8. Study how mature products solve the same problem and use proven patterns instead of inventing new ones.
