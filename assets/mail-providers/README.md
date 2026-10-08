# 邮箱 favicon SVG

2026-10-08 抓取了 16 个邮箱网站的 favicon。下载网址、来源网页、原图尺寸和 SHA256 见 [sources.json](sources.json)。[index.html](index.html) 可直接打开，对照原图和两种 SVG。

- `originals/`：官网原始文件，保留下载字节；部分 `.ico` 文件实际是 PNG。
- `svg-exact/`：前端采用的原样还原版。按原图颜色区域绘制纯 SVG path，保留颜色、透明度和留白，没有嵌入位图。原尺寸渲染在黑、白背景下的最大通道误差 ≤ 1/255（透明度舍入）。放大会保留原 favicon 的像素轮廓。
- `svg/`：光滑几何／曲线重绘版，适合放大；它们是近似重绘，不保证逐像素一致。

Fastmail、Proton 官网已有 SVG，两种版本均保留官方矢量路径。163、126、Yeah 官网共用相同 favicon，未另行替换为不同品牌图形。

图标通过本地 `assets/mail-providers/*.svg` 路由提供，源码编译进可执行文件，不需要联网。账户的 `platform` 控制品牌图标，未配置时由 `provider` 选择；此设置不影响连接方式或账户 ID。

验证（使用本机已有的 Pillow 和 rsvg-convert）：

```sh
python3 assets/mail-providers/verify.py
```
