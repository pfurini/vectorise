# Third-party notices

`vectorise` is licensed MIT OR Apache-2.0. A **binary** of it also contains
code from the crates below, whose licences carry obligations of their own.
This file is that notice.

215 crates are linked into a release binary. Test-only and build-only
dependencies are excluded: they are not in what you download.

Generated from `cargo tree -e normal` and `cargo metadata`. Regenerate with
`just third-party`.

## What each licence asks of you

| Licence | Obligation when redistributing a binary |
|---|---|
| MIT, ISC, BSD-2-Clause, BSD-3-Clause, Zlib, Apache-2.0, 0BSD, Unlicense, BSL-1.0, Unicode-3.0 | reproduce the copyright notice and licence text. This file, shipped beside the binary, does that. |
| MPL-2.0 | the MPL-covered **files** stay under MPL-2.0, and their source must be available to recipients. It does not reach our code or the rest of the binary (MPL-2.0 §3.3, "Larger Work"). Sources are public on crates.io and at the repositories listed below. |

No crate in the graph is GPL, LGPL, or AGPL. `cargo deny check licenses`
enforces the allow-list in `deny.toml` on every CI run.

## MPL-2.0 crates, and where their source is

| Crate | Version | Source |
|---|---|---|
| cssparser | 0.37.0 | https://github.com/servo/rust-cssparser |
| cssparser-color | 0.5.0 | https://github.com/servo/rust-cssparser |
| cssparser-macros | 0.7.1 | https://github.com/servo/rust-cssparser |
| dtoa-short | 0.3.5 | https://github.com/upsuper/dtoa-short |
| lightningcss | 1.0.0-alpha.72 | https://github.com/parcel-bundler/lightningcss |
| lightningcss-derive | 1.0.0-alpha.43 | https://github.com/parcel-bundler/lightningcss |
| parcel_selectors | 0.28.3 | https://github.com/parcel-bundler/lightningcss |
| selectors | 0.39.0 | https://github.com/servo/stylo |

## Everything linked, by licence

### MIT OR Apache-2.0

103 crate(s): ahash 0.8.12, allocator-api2 0.2.21, anstream 1.0.0, anstyle 1.0.14, anstyle-parse 1.0.0, anstyle-query 1.1.5, anyhow 1.0.104, arrayvec 0.7.8, base64 0.22.1, bitflags 2.13.2, cfg-if 1.0.5, clap 4.6.7, clap_builder 4.6.7, clap_derive 4.6.7, clap_lex 1.1.1, colorchoice 1.0.5, crc32fast 1.5.2, crossbeam-deque 0.8.8, crossbeam-epoch 0.9.21, crossbeam-utils 0.8.23, data-url 0.3.2, derive-where 1.7.0, dtoa 1.0.11, either 1.18.0, errno 0.3.14, fdeflate 0.3.7, flate2 1.1.10, geo 0.33.1, geo-types 0.7.20, getrandom 0.3.4, getrandom 0.4.3, gif 0.14.2, glob 0.3.4, half 2.7.1, hash32 0.3.1, hashbrown 0.14.5, hashbrown 0.16.1, hashbrown 0.17.1, heapless 0.8.0, heck 0.4.1, heck 0.5.0, i_overlay 4.5.2, i_overlay 7.0.3, image 0.25.10, image-webp 0.2.4, is_terminal_polyfill 1.70.2, itertools 0.14.0, itertools 0.15.0, itoa 1.0.18, lazy_static 1.5.0, libc 0.2.189, lock_api 0.4.14, log 0.4.34, num-traits 0.2.19, nunny 0.2.2, once_cell 1.21.4, ouroboros 0.18.5, ouroboros_macro 0.18.5, parking_lot_core 0.9.12, paste 1.0.15, pastey 0.1.1, png 0.17.16, png 0.18.1, polycool 0.4.0, proc-macro2 1.0.107, quote 1.0.47, rand 0.10.3, rand_core 0.10.1, rand_pcg 0.10.2, rayon 1.12.0, rayon-core 1.13.0, regex 1.13.1, regex-automata 0.4.18, regex-syntax 0.8.11, robust 1.2.0, roxmltree 0.20.0, roxmltree 0.21.1, rstar 0.12.2, scopeguard 1.2.0, serde 1.0.229, serde_core 1.0.229, serde_derive 1.0.229, serde_json 1.0.151, servo_arc 0.4.3, sif-itree 0.4.1, smallvec 1.16.1, spade 2.15.1, stable_deref_trait 1.2.1, static_assertions 1.1.0, syn 1.0.109, syn 2.0.119, syn 3.0.6, tempfile 3.27.0, terminal_size 0.4.4, thiserror 2.0.20, thiserror-impl 2.0.20, thread_local 1.1.10, unicode-segmentation 1.13.3, unicode-xid 0.2.6, visioncortex 0.9.3, vtracer 1.0.0-alpha.4, weezl 0.1.12, yansi 1.0.1.

### MIT

57 crate(s): aliasable 0.1.3, color_quant 1.1.0, const-str 1.1.0, convert_case 0.6.0, dashmap 6.2.1, data-encoding 2.11.1, derive_more 2.1.1, derive_more-impl 2.1.1, fax 0.2.7, float-cmp 0.9.0, float_next_after 2.0.0, geographiclib-rs 0.2.7, i_float 1.16.0, i_float 3.0.0, i_key_sort 0.10.3, i_key_sort 0.11.0, i_shape 1.18.0, i_shape 3.0.0, i_tree 0.18.0, i_tree 0.19.0, imagesize 0.13.0, libm 0.2.16, matchers 0.2.0, new_debug_unreachable 1.0.6, nu-ansi-term 0.50.3, oxvg_ast 0.0.8, oxvg_collections 0.0.8, oxvg_optimiser 0.0.8, oxvg_parse 0.0.8, oxvg_path 0.0.8, oxvg_serialize 0.0.8, phf 0.11.3, phf 0.13.1, phf 0.14.0, phf_generator 0.13.1, phf_generator 0.14.0, phf_macros 0.13.1, phf_macros 0.14.0, phf_shared 0.11.3, phf_shared 0.13.1, phf_shared 0.14.0, pico-args 0.5.0, precomputed-hash 0.1.1, rgb 0.8.53, sharded-slab 0.1.7, simd-adler32 0.3.10, strict-num 0.1.1, strsim 0.11.1, tiff 0.11.3, tracing 0.1.44, tracing-attributes 0.1.31, tracing-core 0.1.36, tracing-subscriber 0.3.23, typed-arena 2.0.2, urlencoding 2.1.3, xmlwriter 0.1.0, zmij 1.0.23.

### Apache-2.0 OR MIT

12 crate(s): equivalent 1.0.2, fastrand 2.5.0, indexmap 2.14.2, kurbo 0.11.3, kurbo 0.13.1, pin-project-lite 0.2.17, resvg 0.45.1, rustc-hash 2.1.3, simplecss 0.2.2, svgtypes 0.15.3, usvg 0.45.1, utf8parse 0.2.2.

### MIT/Apache-2.0

8 crate(s): bit-vec 0.6.3, bitflags 1.3.2, itertools 0.10.5, itertools 0.8.2, pathdiff 0.2.3, proc-macro2-diagnostics 0.10.1, quick-error 2.0.1, siphasher 1.0.3.

### MPL-2.0

8 crate(s): cssparser 0.37.0, cssparser-color 0.5.0, cssparser-macros 0.7.1, dtoa-short 0.3.5, lightningcss 1.0.0-alpha.72, lightningcss-derive 1.0.0-alpha.43, parcel_selectors 0.28.3, selectors 0.39.0.

### Unlicense OR MIT

4 crate(s): aho-corasick 1.1.5, byteorder 1.5.0, byteorder-lite 0.1.0, memchr 2.8.3.

### Apache-2.0

3 crate(s): approx 0.5.1, flo_curves 0.3.1, flo_curves 0.8.1.

### BSD-2-Clause

3 crate(s): arrayref 0.3.9, roots 0.0.6, roots 0.0.8.

### BSD-2-Clause OR Apache-2.0 OR MIT

2 crate(s): zerocopy 0.8.57, zerocopy-derive 0.8.57.

### BSD-3-Clause

2 crate(s): tiny-skia 0.11.4, tiny-skia-path 0.11.4.

### BSD-3-Clause OR Apache-2.0

2 crate(s): moxcms 0.8.1, pxfm 0.1.30.

### MIT OR Apache-2.0 OR Zlib

2 crate(s): zune-core 0.5.3, zune-jpeg 0.5.15.

### MIT OR Zlib OR Apache-2.0

2 crate(s): miniz_oxide 0.8.9, miniz_oxide 0.9.1.

### (MIT OR Apache-2.0) AND Unicode-3.0

1 crate(s): unicode-ident 1.0.26.

### 0BSD OR MIT OR Apache-2.0

1 crate(s): adler2 2.0.1.

### Apache-2.0 OR BSL-1.0

1 crate(s): ryu 1.0.23.

### Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT

1 crate(s): rustix 1.1.5.

### ISC

1 crate(s): earcut 0.4.5.

### Zlib

1 crate(s): foldhash 0.2.0.

### Zlib OR Apache-2.0 OR MIT

1 crate(s): bytemuck 1.25.2.

