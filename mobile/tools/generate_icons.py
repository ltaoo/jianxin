#!/usr/bin/env python3
"""Draw the native envelope app icon from design colors; standard library only."""
from pathlib import Path
import json
import struct
import zlib
root = Path(__file__).resolve().parents[1]
colors = json.loads((root.parent / 'webpc/.design').read_text())['tokens']['color']
size = 1024
background = bytes.fromhex(colors['primary'][1:])
foreground = bytes.fromhex(colors['primaryForeground'][1:])
pixels = bytearray(background * (size * size))
# Same envelope silhouette as the Android vector; iOS applies its own rounded mask.
segments = [(192,298,832,298),(832,298,832,746),(832,746,192,746),(192,746,192,298),(192,298,512,554),(512,554,832,298)]
for x0,y0,x1,y1 in segments:
    dx,dy=x1-x0,y1-y0
    for y in range(max(0,min(y0,y1)-28),min(size,max(y0,y1)+29)):
        for x in range(max(0,min(x0,x1)-28),min(size,max(x0,x1)+29)):
            t=max(0,min(1,((x-x0)*dx+(y-y0)*dy)/(dx*dx+dy*dy)))
            if (x-x0-t*dx)**2+(y-y0-t*dy)**2 <= 26**2:
                index=3*(y*size+x);pixels[index:index+3]=foreground
scanlines=b''.join(b'\0'+pixels[3*y*size:3*(y+1)*size] for y in range(size))
def chunk(name,data):
    return struct.pack('>I',len(data))+name+data+struct.pack('>I',zlib.crc32(name+data)&0xffffffff)
png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',size,size,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(scanlines,9))+chunk(b'IEND',b'')
assets=root.parent/'ios/Jianxin/Assets.xcassets'
icon=assets/'AppIcon.appiconset';icon.mkdir(parents=True,exist_ok=True)
(assets/'Contents.json').write_text(json.dumps({'info':{'author':'xcode','version':1}},indent=2)+'\n')
(icon/'app-icon.png').write_bytes(png)
(icon/'Contents.json').write_text(json.dumps({'images':[{'filename':'app-icon.png','idiom':'universal','platform':'ios','size':'1024x1024'}],'info':{'author':'xcode','version':1}},indent=2)+'\n')
