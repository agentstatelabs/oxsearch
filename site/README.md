# The oxsearch site

The source of https://oxsearch.agentstatelabs.com, built with
[Astro](https://astro.build) and [Starlight](https://starlight.astro.build).
Pull requests that change it are built and link-checked by
`.github/workflows/site.yml`, and main is published to GitHub Pages.

```bash
npm ci
npm run dev        # http://localhost:4321/
npm run build && node scripts/check-links.mjs
```

## Code on pages comes from the repository

Pages never contain Rust code or dependency lists of their own. They show
parts of the examples in `crates/oxsearch/examples`, which CI compiles and
runs against Elasticsearch:

```mdx
import Example from '../../components/Example.astro';

<Example file="crates/oxsearch/examples/rag.rs" />                    {/* the file, without its //! documentation */}
<Example file="crates/oxsearch/examples/rag.rs" region="retrieve" />  {/* between // ANCHOR: retrieve and // ANCHOR_END: retrieve */}
<Example file="crates/oxsearch/examples/rag.rs" doc="toml" />         {/* the first ```toml block in its //! documentation */}
```

Each example's dependency list, the ```toml block in its documentation, is
checked by `crates/oxsearch/tests/dependencies.rs`: oxsearch at this
release's version, features that exist and cover what the example needs,
and every other crate it uses. A region or block that doesn't exist fails
the site's build.

## Pages from the repository's own Markdown

The coverage report (`docs/coverage.md`, which `cargo run -p xtask --
coverage` writes) and the README's version and license tables (between
`<!-- ANCHOR: name -->` and `<!-- ANCHOR_END: name -->`) are rendered as
they are, by `src/components/Repository.astro`; see
`src/content.config.ts`.

## Links

Write links between pages from the site's root, as `/start/`; `base` in
`astro.config.mjs` is added when the site is built.

## The domain

`public/CNAME` names oxsearch.agentstatelabs.com, and `site` in
`astro.config.mjs` matches it. The DNS record, a CNAME to
agentstatelabs.github.io, is set where the domain is managed.
