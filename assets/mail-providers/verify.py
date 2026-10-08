"""Check source provenance, pure SVG paths and fidelity (requires Pillow/rsvg-convert)."""
import json
import subprocess
import tempfile
from hashlib import sha256
from pathlib import Path
from xml.etree import ElementTree as ET

from PIL import Image, ImageChops


def verify():
    root = Path(__file__).parent
    providers = json.loads((root / 'sources.json').read_text())['providers']
    with tempfile.TemporaryDirectory() as work:
        for provider in providers:
            original = root / provider['original']
            assert sha256(original.read_bytes()).hexdigest() == provider['sha256']
            for key in ['svg', 'exact_svg']:
                node = ET.parse(root / provider[key]).getroot()
                assert node.get('viewBox')
                for child in node.iter():
                    assert child.get('data-n'), (provider['id'], child.tag)
                    assert child.tag.split('}')[-1] not in ['image', 'script', 'foreignObject']
                    assert not any('://' in value or value.startswith('data:') for value in child.attrib.values())
            size = str(provider['size'])
            target = Path(work) / 'redrawn.png'
            subprocess.run(['rsvg-convert', '-w', size, '-h', size, str(root / provider['exact_svg']), '-o', str(target)], check=True)
            if original.suffix == '.svg':
                source = Path(work) / 'source.png'
                subprocess.run(['rsvg-convert', '-w', size, '-h', size, str(original), '-o', str(source)], check=True)
            else:
                source = original
            reference = Image.open(source).convert('RGBA')
            redrawn = Image.open(target).convert('RGBA')
            assert reference.size == redrawn.size
            errors = []
            for color in ['white', 'black']:
                background = Image.new('RGBA', reference.size, color)
                expected = Image.alpha_composite(background, reference).convert('RGB')
                actual = Image.alpha_composite(background, redrawn).convert('RGB')
                errors.append(max(high for low, high in ImageChops.difference(expected, actual).getextrema()))
            # SVG renderers round premultiplied alpha; permit one 8-bit color step.
            assert max(errors) <= 1, (provider['id'], errors)
            print(f"{provider['id']}: max channel difference {max(errors)}/255")
    print(f'{len(providers)} favicon pairs verified.')


if __name__ == '__main__':
    verify()
