"""Compile the web client's checked-in provider SVGs into offline Android vectors."""
from pathlib import Path
import re
import xml.etree.ElementTree as ET

root = Path(__file__).resolve().parents[1]
android = 'http://schemas.android.com/apk/res/android'
aapt = 'http://schemas.android.com/aapt'
ET.register_namespace('android', android)
ET.register_namespace('aapt', aapt)
def attr(name): return '{' + android + '}' + name
def color(value):
    if value.lower() == '#fff': return '#FFFFFF'
    return value
output = root / 'android/app/src/main/res/drawable'
for source in sorted((root.parent / 'assets/mail-providers/svg-exact').glob('*.svg')):
    svg = ET.parse(source).getroot()
    x, y, width, height = map(float, svg.attrib['viewBox'].split())
    assert x == 0 and y == 0
    vector = ET.Element('vector', {attr('width'): '32dp', attr('height'): '32dp', attr('viewportWidth'): f'{width:g}', attr('viewportHeight'): f'{height:g}'})
    gradients = {node.get('id'): node for node in svg.iter() if node.tag.endswith('Gradient')}
    for node in svg.iter():
        tag = node.tag.split('}')[-1]
        if tag == 'path': data = node.attrib['d']
        elif tag == 'circle':
            cx, cy, radius = (float(node.attrib[key]) for key in ['cx', 'cy', 'r'])
            data = f'M{cx-radius:g},{cy:g}a{radius:g},{radius:g} 0,1 0,{2*radius:g},0a{radius:g},{radius:g} 0,1 0,{-2*radius:g},0'
        else: continue
        assert not node.get('transform'), 'Unsupported transformed provider path'
        path = ET.SubElement(vector, 'path', {attr('pathData'): data})
        fill = node.get('fill', '#000000')
        if fill.startswith('url('):
            paint = gradients[fill[5:-1]]
            assert paint.get('gradientUnits') == 'userSpaceOnUse'
            wrapper = ET.SubElement(path, '{' + aapt + '}attr', {'name': 'android:fillColor'})
            if paint.tag.endswith('linearGradient'):
                spec = {attr('type'): 'linear', attr('startX'): paint.get('x1', '0'), attr('startY'): paint.get('y1', '0'), attr('endX'): paint.get('x2', '0'), attr('endY'): paint.get('y2', '0')}
            else:
                matrix = list(map(float,re.findall(r'[-\d.]+',paint.get('gradientTransform',''))))
                assert len(matrix) == 6 and matrix[1] == 0 and matrix[2] == 0
                # VectorDrawable has circular radial gradients; keep the original light center.
                spec = {attr('type'): 'radial',attr('centerX'): str(matrix[4]),attr('centerY'): str(matrix[5]),attr('gradientRadius'): str((matrix[0]+matrix[3])/2)}
            gradient = ET.SubElement(wrapper, 'gradient', spec)
            for stop in paint:
                ET.SubElement(gradient,'item',{attr('offset'): stop.get('offset','0'),attr('color'): color(stop.attrib['stop-color'])})
        else: path.set(attr('fillColor'),color(fill))
        if node.get('fill-opacity'): path.set(attr('fillAlpha'),node.get('fill-opacity'))
        if node.get('fill-rule'): path.set(attr('fillType'),node.get('fill-rule'))
    ET.indent(vector)
    text = ET.tostring(vector,encoding='unicode')
    (output / f'mail_provider_{source.stem}.xml').write_text('<!-- Generated from assets/mail-providers/svg-exact/' + source.name + ' -->\n' + text + '\n',encoding='utf-8')
