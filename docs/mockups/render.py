#!/usr/bin/env python3
"""Static mockups of the pixbar layout (52x16). Prints ASCII, writes PNGs if Pillow is present.

v2: resting screen is statusline-like text; the effort slider is a temporary overlay with round shapes.
"""
import os

W, H = 52, 16
F3 = {  # 3x5 caps
 'A':"010 101 111 101 101",'B':"110 101 110 101 110",'C':"011 100 100 100 011",'D':"110 101 101 101 110",
 'E':"111 100 110 100 111",'F':"111 100 110 100 100",'G':"011 100 101 101 011",'H':"101 101 111 101 101",
 'I':"111 010 010 010 111",'K':"101 101 110 101 101",'L':"100 100 100 100 111",'M':"101 111 111 101 101",
 'N':"110 101 101 101 101",'O':"111 101 101 101 111",'P':"110 101 110 100 100",'R':"110 101 110 101 101",
 'S':"011 100 010 001 110",'T':"111 010 010 010 010",'U':"101 101 101 101 111",'W':"101 101 111 111 101",
 'X':"101 101 010 101 101",'-':"000 000 111 000 000",' ':"000 000 000 000 000",'·':"000 000 010 000 000",
 '0':"111 101 101 101 111",'1':"010 110 010 010 111",'2':"111 001 111 100 111",'3':"111 001 111 001 111",
 '4':"101 101 111 001 001",'5':"111 100 111 001 111",'6':"111 100 111 101 111",'7':"111 001 001 001 001",
 '8':"111 101 111 101 111",'9':"111 101 111 001 111",'/':"001 001 010 100 100",'%':"101 001 010 100 101",
}
F5 = {  # 5x7 caps (only what the overlays need)
 'A':"01110 10001 10001 11111 10001 10001 10001",'B':"11110 10001 10001 11110 10001 10001 11110",
 'D':"11110 10001 10001 10001 10001 10001 11110",'E':"11111 10000 10000 11110 10000 10000 11111",
 'F':"11111 10000 10000 11110 10000 10000 10000",'G':"01110 10001 10000 10111 10001 10001 01111",
 'H':"10001 10001 10001 11111 10001 10001 10001",'I':"01110 00100 00100 00100 00100 00100 01110",
 'L':"10000 10000 10000 10000 10000 10000 11111",'M':"10001 11011 10101 10101 10001 10001 10001",
 'O':"01110 10001 10001 10001 10001 10001 01110",'P':"11110 10001 10001 11110 10000 10000 10000",
 'S':"01111 10000 10000 01110 00001 00001 11110",'U':"10001 10001 10001 10001 10001 10001 01110",
 'W':"10001 10001 10001 10101 10101 10101 01010",'X':"10001 10001 01010 00100 01010 10001 10001",
}
C = dict(white=(229,229,217), dim=(63,63,63), opus=(255,106,61), fable=(0,200,180), working=(255,150,0),
         blocked=(255,40,40), done=(0,230,90), idle=(0,100,255), ctx=(0,110,230), warn=(255,160,0), off=(0,0,0))
LEVELS = ["LOW", "MED", "HIGH", "XHIGH", "MAX"]
MX0, MX1 = 11, 50                     # main area columns (inclusive)

def new(): return [[C['off']]*W for _ in range(H)]
def px(fb,x,y,c):
    if 0<=x<W and 0<=y<H: fb[y][x]=c
def text(fb,s,x,y,c,font=F3,gap=1):
    w=len(next(iter(font.values())).split()[0])
    for ch in s:
        if ch==' ' and font is F3: x+=2; continue       # narrow space
        for r,row in enumerate(font[ch].split()):
            for k,b in enumerate(row):
                if b=='1': px(fb,x+k,y+r,c)
        x+=w+gap
    return x-gap
def width(s,font=F3,gap=1):
    w=len(next(iter(font.values())).split()[0]); n=0
    for ch in s: n+= 2 if (ch==' ' and font is F3) else w+gap
    return n-gap
def disc(fb,cx,cy,r,c,ring=False):
    shape={1:[1,3,1],2:[3,5,5,5,3],3:[3,5,7,7,7,5,3]}[r]
    for i,wd in enumerate(shape):
        y=cy-r+i
        for x in range(cx-wd//2,cx+wd//2+1):
            edge = i in (0,len(shape)-1) or x in (cx-wd//2,cx+wd//2)
            if not ring or edge: px(fb,x,y,c)

def strip(fb,agents,focus):
    for i,st in enumerate(agents[:8]):
        col,row=divmod(i,4); x0=1+col*4; y0=row*4
        for y in range(3):
            for x in range(3): px(fb,x0+x,y0+y,C[st])
        if i==focus:
            cx = 0 if col==0 else 8
            for y in range(3): px(fb,cx,y0+y,C['white'])

def rest(space,tab,level,model,agents,focus,used_k,win_k,line2=None):
    """Statusline-like resting screen: model + effort / space·tab / context bar."""
    fb=new(); strip(fb,agents,focus)
    x=text(fb,model.upper(),MX0,0,C[model]); text(fb,LEVELS[level],x+3,0,C['white'])
    text(fb,line2 or f"{space}·{tab}",MX0,6,C['dim'] if not line2 else C['white'])
    pct=used_k/win_k; n=max(1,round((MX1-MX0+1)*pct)); col=C['warn'] if pct>=.7 else C['ctx']
    for i in range(MX1-MX0+1):
        if i<n:
            for y in (13,14): px(fb,MX0+i,y,col)
        elif i%4==3: px(fb,MX0+i,14,C['dim'])
    px(fb,MX0,13,C['off']) if n>1 else None       # rounded left cap
    return fb

def slider_beads(level,model,agents,focus):
    """Overlay A: five round beads; passed = filled, ahead = hollow rings, current = big disc."""
    fb=new(); strip(fb,agents,focus); hue=C[model]
    word=LEVELS[level]; text(fb,word,MX0+(40-width(word,F5))//2,0,C['white'],F5)
    for i in range(5):
        cx=MX0+3+i*8+1
        if i<level:   disc(fb,cx,12,2,hue)
        elif i>level: disc(fb,cx,12,2,C['dim'],ring=True)
    for i in range(level):                         # string between passed beads
        for x in range(MX0+3+i*8+4, MX0+3+(i+1)*8-1): px(fb,x,12,hue)
    disc(fb,MX0+3+level*8+1,12,3,hue); px(fb,MX0+3+level*8,10,C['white'])   # big disc + glint
    return fb

def slider_rail(level,model,agents,focus):
    """Overlay B: capsule rail with a round knob; detents are pin-holes in the fill."""
    fb=new(); strip(fb,agents,focus); hue=C[model]
    word=LEVELS[level]; text(fb,word,MX0+(40-width(word,F5))//2,0,C['white'],F5)
    x0,x1=MX0+1,MX1-1; kx=x0+3+level*8
    for x in range(x0,kx):
        for y in (11,12,13):
            if x==x0 and y!=12: continue           # rounded cap
            px(fb,x,y,hue)
    for i in range(5):
        cx=x0+3+i*8
        if cx<kx: px(fb,cx,12,C['off'])
        elif cx>kx: disc(fb,cx,12,1,C['dim'])
    disc(fb,kx,12,3,C['white']); disc(fb,kx,12,2,hue)   # white-rimmed knob
    return fb

def model_flip(model,agents,focus):
    fb=new(); strip(fb,agents,focus); word=model.upper(); x=MX0+(40-width(word,F5))//2
    text(fb,word,x,2,C['white'],F5)
    for i in range(width(word,F5)):
        for y in (12,13): px(fb,x+i,y,C[model])
    px(fb,x,12,C['off']); px(fb,x+width(word,F5)-1,12,C['off'])
    return fb

def ascii(fb):
    return '\n'.join(''.join('.' if p==C['off'] else ':' if p==C['dim'] else '#' for p in row) for row in fb)
def png(fb,path,scale=14):
    try: from PIL import Image, ImageDraw
    except ImportError: return False
    im=Image.new('RGB',(W*scale,H*scale),(12,12,12)); d=ImageDraw.Draw(im)
    for y,row in enumerate(fb):
        for x,p in enumerate(row):
            d.ellipse([x*scale+2,y*scale+2,(x+1)*scale-2,(y+1)*scale-2],fill=p if p!=C['off'] else (24,24,24))
    im.save(path); return True

AG=['working','blocked','idle','done','working']
frames={
 '1-rest':                rest("WEB","2",3,"opus",AG,0,104,1000),
 '2-rest-context-number': rest("PARSER","1",1,"fable",AG,2,742,1000,line2="742K 74%"),
 '3-effort-beads':        slider_beads(3,'opus',AG,0),
 '4-effort-rail':         slider_rail(3,'opus',AG,0),
 '5-model-flip':          model_flip('fable',AG,0),
}
here=os.path.dirname(os.path.abspath(__file__))
for f in os.listdir(here):
    if f.endswith('.png'): os.remove(os.path.join(here,f))
ok=False
for k,fb in frames.items():
    print(f"--- {k}"); print(ascii(fb)); ok=png(fb,os.path.join(here,k+'.png'))
print("png written" if ok else "Pillow not installed: ASCII only")
