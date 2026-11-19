# 属性参考

> **本文由 `zstylec props` 输出 + 手工头注维护。**
>
> 重新生成属性列表：
>
> ```text
> cargo run --manifest-path ../ZephyraCLI/Cargo.toml -p zstylec-bin -- props
> ```

全部 53 个受支持的样式属性（以 `zstylec props` 为准）。

完整的属性语义、取值范围与对应 Rust 属性键见
[`zstylec/src/props_map.rs`](../../../zstylec/src/props_map.rs) —— 那是**唯一**的映射表。

写错属性名会报错并给出最接近的几个，见[样式表](../04-stylesheets.md)。定位/布局语义见[布局与定位规范](../layout-spec.md)。

```text
align-content
align-items
align-self
anchor
background
blend-mode
border-color
border-width
collapsed
corner-radius
corner-shape
cross-align
decoration
direction
disabled
filter
flex-basis
flex-direction
flex-grow
flex-shrink
flex-wrap
gap
height
icon-color
icon-name
image-size
image-src
image-texture
image-tint
interactive
justify-content
layer
left
main-align
material
material-params
opacity
order
padding
position
scale-x
scale-y
selected
spacing
text
text-color
text-line-height
text-max-width
text-size
top
translate-x
translate-y
width
```
