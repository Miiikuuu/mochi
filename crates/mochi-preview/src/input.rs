use mochi_core::{MateEvent, SceneContent, Vec2};
use mochi_host::{Host, InstanceHandle};
use std::fmt;

pub const DRAG_THRESHOLD: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError(pub String);
impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for InputError {}

#[derive(Debug, Clone, Copy)]
pub enum PointerInput {
    Move(Vec2),
    Press(u16),
    Release(u16),
    Leave,
    /// Focus loss, suspension, resize/DPI change or shutdown. Ends drag without click.
    Cancel,
    /// Re-evaluate hover after simulation/z-order changes, without inventing movement.
    Refresh,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedEvent {
    pub target: InstanceHandle,
    pub event: MateEvent,
}

#[derive(Clone)]
struct Press {
    target: InstanceHandle,
    button: u16,
    origin: Vec2,
    last: Vec2,
    // Snapshot at press: host layout may move during dragging, without changing
    // the coordinate basis seen by the gesture's Behavior events.
    offset: Vec2,
    dragging: bool,
    threshold: f64,
}
#[derive(Clone)]
pub struct PointerRouter {
    size: Vec2,
    position: Option<Vec2>,
    hover: Option<InstanceHandle>,
    press: Option<Press>,
    drag_threshold: f64,
}
impl PointerRouter {
    pub fn new(width: u32, height: u32) -> Result<Self, InputError> {
        if width == 0 || height == 0 {
            return Err(InputError("canvas must have positive size".into()));
        }
        Ok(Self {
            size: Vec2::new(f64::from(width), f64::from(height)),
            position: None,
            hover: None,
            press: None,
            drag_threshold: DRAG_THRESHOLD,
        })
    }
    pub fn hovered(&self) -> Option<&InstanceHandle> {
        self.hover.as_ref()
    }
    /// Threshold in canvas units, captured at the start of each new press.
    /// Invalid values leave the router and any active gesture unchanged.
    pub fn set_drag_threshold(&mut self, threshold: f64) -> Result<(), InputError> {
        if !threshold.is_finite() || threshold <= 0.0 {
            return Err(InputError(
                "drag threshold must be finite and positive".into(),
            ));
        }
        self.drag_threshold = threshold;
        Ok(())
    }
    pub fn dragging(&self) -> Option<&InstanceHandle> {
        self.press
            .as_ref()
            .filter(|p| p.dragging)
            .map(|p| &p.target)
    }
    /// Pure routing with atomic router updates. The caller dispatches returned
    /// events in order and inspects every HostStep, including capability requests.
    pub fn route(
        &mut self,
        host: &Host,
        input: PointerInput,
    ) -> Result<Vec<RoutedEvent>, InputError> {
        let mut next = self.clone();
        let mut out = Vec::new();
        if next.hover.as_ref().is_some_and(|h| host.get(h).is_err()) {
            next.hover = None;
        }
        if next
            .press
            .as_ref()
            .is_some_and(|p| host.get(&p.target).is_err())
        {
            next.press = None;
        }
        match input {
            PointerInput::Move(point) => {
                point.validate().map_err(|e| InputError(e.to_string()))?;
                next.position = Some(point);
                if let Some(press) = &mut next.press {
                    press.last = point;
                    let distance = (point.x - press.origin.x).hypot(point.y - press.origin.y);
                    if press.button == 1 && !press.dragging && distance >= press.threshold {
                        press.dragging = true;
                        push(
                            &mut out,
                            &press.target,
                            MateEvent::DragStart {
                                position: local(press.origin, press.offset)?,
                            },
                        );
                    }
                    if press.dragging {
                        push(
                            &mut out,
                            &press.target,
                            MateEvent::DragMove {
                                position: local(point, press.offset)?,
                            },
                        );
                    }
                } else {
                    next.refresh_hover(host, &mut out)?;
                }
            }
            PointerInput::Press(button) => {
                if next.press.is_none() {
                    next.refresh_hover(host, &mut out)?;
                    if let (Some(target), Some(origin)) = (&next.hover, next.position) {
                        let offset = host
                            .get(target)
                            .map_err(|e| InputError(e.to_string()))?
                            .presentation()
                            .offset;
                        local(origin, offset)?;
                        next.press = Some(Press {
                            target: target.clone(),
                            button,
                            origin,
                            last: origin,
                            offset,
                            dragging: false,
                            threshold: next.drag_threshold,
                        });
                    }
                }
            }
            PointerInput::Release(button) => {
                if next.press.as_ref().is_some_and(|p| p.button == button) {
                    let press = next.press.take().expect("matching button");
                    if press.dragging {
                        push(
                            &mut out,
                            &press.target,
                            MateEvent::DragEnd {
                                position: local(press.last, press.offset)?,
                            },
                        );
                    } else if let Some(point) = next.position {
                        if next.hit(host, point)?.as_ref() == Some(&press.target) {
                            push(
                                &mut out,
                                &press.target,
                                MateEvent::PointerClick {
                                    position: local(point, press.offset)?,
                                    button,
                                },
                            );
                        }
                    }
                    next.refresh_hover(host, &mut out)?;
                }
            }
            PointerInput::Leave => {
                next.position = None;
                next.set_hover(None, &mut out);
                // Native pointer grab may continue reporting movement/release.
                // Focus loss separately cancels an unfinished gesture.
            }
            PointerInput::Cancel => {
                if let Some(press) = next.press.take().filter(|p| p.dragging) {
                    push(
                        &mut out,
                        &press.target,
                        MateEvent::DragEnd {
                            position: local(press.last, press.offset)?,
                        },
                    );
                }
                next.position = None;
                next.set_hover(None, &mut out);
            }
            PointerInput::Refresh => {
                if next.press.is_none() {
                    next.refresh_hover(host, &mut out)?;
                }
            }
        }
        *self = next;
        Ok(out)
    }
    fn refresh_hover(&mut self, host: &Host, out: &mut Vec<RoutedEvent>) -> Result<(), InputError> {
        let target = self
            .position
            .map(|p| self.hit(host, p))
            .transpose()?
            .flatten();
        self.set_hover(target, out);
        Ok(())
    }
    fn set_hover(&mut self, target: Option<InstanceHandle>, out: &mut Vec<RoutedEvent>) {
        if self.hover == target {
            return;
        }
        if let Some(old) = self.hover.take() {
            push(out, &old, MateEvent::PointerLeave);
        }
        if let Some(new) = &target {
            push(out, new, MateEvent::PointerEnter);
        }
        self.hover = target;
    }
    fn hit(&self, host: &Host, p: Vec2) -> Result<Option<InstanceHandle>, InputError> {
        if p.x < 0.0 || p.y < 0.0 || p.x >= self.size.x || p.y >= self.size.y {
            return Ok(None);
        }
        let scenes = host.scenes().map_err(|e| InputError(e.to_string()))?;
        for layer in scenes.iter().rev() {
            for node in layer.scene.nodes.iter().rev() {
                let (size, image) = match &node.content {
                    SceneContent::Sprite { asset, size, .. } => (
                        size,
                        Some(
                            layer
                                .package
                                .image(asset)
                                .map_err(|e| InputError(e.to_string()))?,
                        ),
                    ),
                    // The ordinary preview uses the declared camera rectangle for
                    // skeletal input. Desktop shells hit-test their rendered alpha.
                    SceneContent::Skeletal { asset, size, .. } => {
                        layer
                            .package
                            .spine(asset)
                            .map_err(|e| InputError(e.to_string()))?;
                        (size, None)
                    }
                    _ => {
                        return Err(InputError(format!(
                            "unsupported hit-test content in {}",
                            node.id
                        )))
                    }
                };
                if node.opacity == 0.0
                    || node.transform.scale.x == 0.0
                    || node.transform.scale.y == 0.0
                {
                    continue;
                }
                let sx = size.x * node.transform.scale.x;
                let sy = size.y * node.transform.scale.y;
                if !sx.is_finite() || !sy.is_finite() || sx == 0.0 || sy == 0.0 {
                    return Err(InputError("unrepresentable hit-test transform".into()));
                }
                let (sin, cos) = node.transform.rotation_radians.sin_cos();
                let dx = p.x - node.transform.translation.x;
                let dy = p.y - node.transform.translation.y;
                let u = (cos * dx + sin * dy) / sx;
                let v = (-sin * dx + cos * dy) / sy;
                if !u.is_finite() || !v.is_finite() {
                    return Err(InputError("unrepresentable hit-test point".into()));
                }
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let Some(image) = image else {
                    return Ok(host.find(layer.instance_id));
                };
                let x = ((u * f64::from(image.width())) as u32).min(image.width() - 1);
                let y = ((v * f64::from(image.height())) as u32).min(image.height() - 1);
                let alpha =
                    image.rgba8()[(y as usize * image.width() as usize + x as usize) * 4 + 3];
                if (f64::from(alpha) * node.opacity).round() > 0.0 {
                    return Ok(host.find(layer.instance_id));
                }
            }
        }
        Ok(None)
    }
}
fn local(p: Vec2, offset: Vec2) -> Result<Vec2, InputError> {
    let p = Vec2::new(p.x - offset.x, p.y - offset.y);
    p.validate().map_err(|e| InputError(e.to_string()))?;
    Ok(p)
}
fn push(out: &mut Vec<RoutedEvent>, target: &InstanceHandle, event: MateEvent) {
    out.push(RoutedEvent {
        target: target.clone(),
        event,
    });
}
