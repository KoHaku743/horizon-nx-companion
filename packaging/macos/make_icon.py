# Draws icon.png (the app icon, after the Switch build's icon) and crates/app/icon-128.rgba (the
# window's). Needs Pillow and the Geist SemiBold font beside it (SIL OFL). Run from packaging/macos.
from PIL import Image, ImageDraw, ImageFont, ImageFilter
S=1024
img=Image.new("RGBA",(S,S),(0,0,0,0))
# macOS app icon grid: an 824 px rounded square, centred, with a soft shadow.
box=(100,100,924,924); r=185
shadow=Image.new("RGBA",(S,S),(0,0,0,0))
ImageDraw.Draw(shadow).rounded_rectangle((100,112,924,936),r,fill=(0,0,0,110))
img=Image.alpha_composite(img,shadow.filter(ImageFilter.GaussianBlur(14)))
card=Image.new("RGBA",(S,S),(0,0,0,0))
d=ImageDraw.Draw(card)
d.rounded_rectangle(box,r,fill=(22,26,34,255))
# The Switch icon's motif, scaled into the square: an H with the horizon through it.
u=824/256; ox=oy=100
def R(x0,y0,x1,y1,c): d.rectangle((ox+x0*u,oy+y0*u,ox+x1*u,oy+y1*u),fill=c)
light=(236,237,242,255); grey=(104,107,117,255); purple=(155,135,245,255)
for x in (64,160):
    R(x,46,x+32,120,light)
    R(x,120,x+32,190,grey)
R(0,112,256,128,purple)
# The pill: "NX".
pw,ph=86,40; px=(256-pw)/2; py=198
d.rounded_rectangle((ox+px*u,oy+py*u,ox+(px+pw)*u,oy+(py+ph)*u),ph*u/2,fill=purple)
f=ImageFont.truetype("Geist-SemiBold.ttf",int(30*u))
cx=ox+128*u; cy=oy+(py+ph/2)*u
d.text((cx,cy+2),"NX",font=f,fill=(22,26,34,255),anchor="mm")
# Keep everything inside the rounded square.
mask=Image.new("L",(S,S),0); ImageDraw.Draw(mask).rounded_rectangle(box,r,fill=255)
img.paste(card,(0,0),mask)
img.save("icon.png")
small=img.resize((128,128),Image.LANCZOS)
open("../../crates/app/icon-128.rgba","wb").write(small.tobytes())

