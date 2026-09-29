"""Pure layout rules shared by the native bubble and its tests (logical pixels)."""
def tail_attachment(anchor, card, inset=24):
    """Facing edge and offset along it for a tail aimed at the character.

    Work in desktop logical coordinates, after scrolling/animation/clamping.
    Choose the closest facing edge, keeping the tail base clear of rounded
    corners. Overlapping targets have no tail.
    """
    ax, ay, aw, ah = anchor
    x, y, w, h = card
    tx, ty = ax+aw/2, ay+ah/2
    if w <= 0 or h <= 0 or (x <= tx <= x+w and y <= ty <= y+h):
        return None
    def clamp(value,length):
        margin = min(inset,length/2)
        return round(max(margin,min(value,length-margin)))
    ox,oy = clamp(tx-x,w),clamp(ty-y,h)
    candidates = []
    for side,facing,offset,px,py in [
        ('left',tx<x,oy,x,y+oy), ('right',tx>x+w,oy,x+w,y+oy),
        ('top',ty<y,ox,x+ox,y), ('bottom',ty>y+h,ox,x+ox,y+h)]:
        if facing:candidates.append(((tx-px)**2+(ty-py)**2,side,offset))
    _,side,offset = min(candidates)
    return side,offset


def place(anchor, bubble, work, prefer_sides=False, left_limit=None):
    ax, ay, aw, ah = anchor
    bw, bh = bubble
    x, y, w, h = work
    gap = 12
    # Prefer above the character, then its right, left, or below.
    choices = [(ax + (aw-bw)//2, ay-bh-gap, True),
               (ax+aw+gap, ay+(ah-bh)//2, False),
               (min(ax, left_limit)-bw-gap if left_limit is not None else ax-bw-gap,
                ay+(ah-bh)//2, False),
               (ax+(aw-bw)//2, ay+ah+gap, True)]
    if prefer_sides:choices = [choices[1], choices[2], choices[0], choices[3]]
    for bx, by, horizontal in choices:
        # Slide along the chosen side before rejecting it. At a screen corner,
        # a slightly off-screen centered panel can still fit beside the primary.
        # Falling straight back to a clamped "above" position would cover it.
        if horizontal:
            bx = max(x, min(bx, x+max(0, w-bw)))
        else:
            by = max(y, min(by, y+max(0, h-bh)))
        if x <= bx and y <= by and bx+bw <= x+w and by+bh <= y+h:
            return int(bx), int(by)
    bx, by, _ = choices[0]
    return int(max(x, min(bx, x+max(0, w-bw)))), int(max(y, min(by, y+max(0, h-bh))))


def speech_place(anchor, bubble, work, gap=6, tail=12, left_limit=None):
    """Keep the frame on-screen and return the tail side and its edge offset."""
    ax, ay, aw, ah = anchor
    bw, bh = bubble
    wx, wy, ww, wh = work
    cx, cy = ax+aw/2, ay+ah/2
    candidates = [(ax+aw+gap, cy-bh/2, 'left'),
                  (min(ax-bw-gap, left_limit-bw-max(gap, 12)) if left_limit is not None else ax-bw-gap,
                   cy-bh/2, 'right'),
                  (cx-bw/2, ay-bh-gap, 'bottom'),
                  (cx-bw/2, ay+ah+gap, 'top')]
    chosen = None
    for x, y, side in candidates:
        # Clamp along the attachment edge before testing, so a character near
        # a corner can still have a bubble above with an offset pointing tail.
        if side in ('top', 'bottom'):
            x = max(wx, min(x, wx+max(0, ww-bw)))
        else:
            y = max(wy, min(y, wy+max(0, wh-bh)))
        if wx <= x and wy <= y and x+bw <= wx+ww and y+bh <= wy+wh:
            chosen = x, y, side
            break
    if chosen is None:
        x, y, side = candidates[0]
        chosen = max(wx, min(x, wx+max(0, ww-bw))), max(wy, min(y, wy+max(0, wh-bh))), side
    x, y, side = chosen
    point, length = (cx-x, bw) if side in ('top', 'bottom') else (cy-y, bh)
    inset = tail+26
    offset = max(inset, min(point, max(inset, length-inset)))
    return (round(x), round(y)), side, round(offset)
