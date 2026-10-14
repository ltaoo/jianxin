#!/usr/bin/env python3
"""Generate every app icon from SVG; requires rsvg-convert and ImageMagick."""
import argparse
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'assets'
SVG = 'http://www.w3.org/2000/svg'
ET.register_namespace('', SVG)
SCENES = ('tray', 'windows-tray', 'macos', 'windows', 'notification', 'android', 'ios', 'web')


def svg_bytes(path):
    data = path.read_bytes()
    node = ET.fromstring(data)
    if node.tag != f'{{{SVG}}}svg' or 'viewBox' not in node.attrib:
        raise ValueError(f'{path}: expected an SVG with a viewBox')
    if any(element.tag == f'{{{SVG}}}script' for element in node.iter()):
        raise ValueError(f'{path}: scripts are not allowed in logos')
    return data


def framed_svg(data, margin, rounded=False):
    outer = ET.Element(f'{{{SVG}}}svg', {'viewBox': '0 0 1024 1024', 'width': '1024', 'height': '1024', 'data-n': 'platform-icon-canvas'})
    group = ET.SubElement(outer, f'{{{SVG}}}g', {'data-n': 'platform-icon-artwork'})
    if rounded:
        defs = ET.SubElement(outer, f'{{{SVG}}}defs', {'data-n': 'platform-icon-definitions'})
        clip = ET.SubElement(defs, f'{{{SVG}}}clipPath', {'id': 'jianxin-platform-tile', 'data-n': 'platform-tile-clip'})
        ET.SubElement(clip, f'{{{SVG}}}path', {'data-n': 'platform-tile-shape', 'd': 'M512 100c172 0 258 0 321 31 27 13 47 33 60 60 31 63 31 149 31 321s0 258-31 321c-13 27-33 47-60 60-63 31-149 31-321 31s-258 0-321-31c-27-13-47-33-60-60-31-63-31-149-31-321s0-258 31-321c13-27 33-47 60-60 63-31 149-31 321-31Z'})
        group.set('clip-path', 'url(#jianxin-platform-tile)')
    inner = ET.fromstring(data)
    inner.attrib.update(x=str(margin), y=str(margin), width=str(1024 - 2 * margin), height=str(1024 - 2 * margin))
    group.append(inner)
    return ET.tostring(outer)


def render(data, size, opaque=False, monochrome=False):
    png = subprocess.run(['rsvg-convert', '-w', str(size), '-h', str(size)], input=data, capture_output=True, check=True).stdout
    if opaque or monochrome:
        options = ['-background', '#D6E7FC', '-alpha', 'remove', '-alpha', 'off'] if opaque else ['-channel', 'RGB', '-evaluate', 'set', '0', '+channel']
        png = subprocess.run(['magick', 'png:-', *options, '-define', 'png:exclude-chunk=date,time', 'png:-'], input=png, capture_output=True, check=True).stdout
    assert png[:8] == b'\x89PNG\r\n\x1a\n' and struct.unpack('>II', png[16:24]) == (size, size)
    return png


def ico(frames):
    offset = 6 + 16 * len(frames)
    entries = []
    for size, png in frames:
        entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(png), offset))
        offset += len(png)
    return struct.pack('<HHH', 0, 1, len(frames)) + b''.join(entries) + b''.join(png for _, png in frames)


def generate(sources):
    outputs = {}
    outputs[ASSETS / 'jianxin-icon.png'] = render(sources['main'], 1024)
    outputs[ASSETS / 'jianxin-macos.png'] = render(framed_svg(sources['macos'], 100, rounded=True), 1024)
    outputs[ASSETS / 'jianxin-tray.png'] = render(sources['tray'], 32)
    outputs[ASSETS / 'jianxin-tray-windows.png'] = render(sources['windows-tray'], 32)
    outputs[ASSETS / 'jianxin-tray-template.png'] = render(framed_svg(sources['tray'], 60), 36, monochrome=True)
    outputs[ASSETS / 'jianxin-notification.png'] = render(sources['notification'], 256)
    outputs[ASSETS / 'jianxin.ico'] = ico([(size, render(sources['windows'], size)) for size in (16, 24, 32, 48, 64, 128, 256)])
    outputs[ASSETS / 'jianxin-web.svg'] = sources['web']
    outputs[ASSETS / 'jianxin-favicon.ico'] = ico([(size, render(sources['web'], size)) for size in (16, 32, 48)])
    outputs[ROOT / 'ios/Jianxin/Assets.xcassets/AppIcon.appiconset/app-icon.png'] = render(sources['ios'], 1024, opaque=True)
    drawable = ROOT / 'android/app/src/main/res/drawable-nodpi'
    outputs[drawable / 'ic_launcher.png'] = render(sources['android'], 192)
    outputs[drawable / 'ic_launcher_foreground.png'] = render(framed_svg(sources['android'], 199), 432)
    outputs[drawable / 'ic_launcher_monochrome.png'] = render(framed_svg(sources['tray'], 199), 432, monochrome=True)
    background = subprocess.run(['magick', 'png:-', '-format', '%[hex:p{0,0}]', 'info:'], input=render(sources['android'], 192, opaque=True), capture_output=True, check=True).stdout.decode()[:6]
    outputs[ROOT / 'android/app/src/main/res/values/app_icon.xml'] = f'<?xml version="1.0" encoding="utf-8"?>\n<resources>\n    <color name="app_icon_background">#{background}</color>\n</resources>\n'.encode()
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, help='replace the shared SVG (default: assets/jianxin-icon.svg)')
    for scene in SCENES:
        parser.add_argument(f'--{scene}-source', type=Path, help=f'persist an SVG override for {scene}')
    parser.add_argument('--reset', action='append', choices=SCENES, default=[], help='remove a scene override; repeat for multiple scenes')
    parser.add_argument('--check', action='store_true', help='verify committed assets without changing files')
    args = parser.parse_args()
    if args.check and (args.source or args.reset or any(getattr(args, f'{scene.replace("-", "_")}_source') for scene in SCENES)):
        parser.error('--check cannot be combined with replacements or resets')
    for tool in ('rsvg-convert', 'magick'):
        if not shutil.which(tool):
            parser.error(f'{tool} is required (macOS: brew install librsvg imagemagick)')
    main_source = svg_bytes(args.source or ASSETS / 'jianxin-icon.svg')
    sources = {'main': main_source}
    replacements = {ASSETS / 'jianxin-icon.svg': main_source} if args.source else {}
    removals = []
    for scene in SCENES:
        override = ASSETS / f'logo-overrides/{scene}.svg'
        source = getattr(args, f'{scene.replace("-", "_")}_source')
        if scene in args.reset and source:
            parser.error(f'--reset {scene} conflicts with --{scene}-source')
        if scene in args.reset:
            removals.append(override)
        if source:
            replacements[override] = svg_bytes(source)
        fallback = sources['tray'] if scene == 'windows-tray' else main_source
        sources[scene] = replacements.get(override) or (svg_bytes(override) if override.exists() and scene not in args.reset else fallback)
    # Render everything before writing; invalid sources cannot leave partially generated assets.
    outputs = generate(sources)
    outputs.update(replacements)
    if args.check:
        stale = [str(path.relative_to(ROOT)) for path, data in outputs.items() if not path.exists() or path.read_bytes() != data]
        if stale:
            parser.exit(1, 'Stale icons; run python3 scripts/generate-icons.py:\n' + '\n'.join(stale) + '\n')
    else:
        for path, data in outputs.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
                temporary.write(data)
                temporary_path = Path(temporary.name)
            temporary_path.chmod(0o644)
            temporary_path.replace(path)
        for path in removals:
            path.unlink(missing_ok=True)
    print(f'{"Verified" if args.check else "Generated"} {len(outputs)} icons: desktop, tray, notifications, Android, iOS and Web.')


if __name__ == '__main__':
    main()
