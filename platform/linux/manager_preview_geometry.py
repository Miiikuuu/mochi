"""Logical preview sizing shared by all GTK4 viewport widths."""
from dataclasses import dataclass


@dataclass(frozen=True)
class PreviewGeometry:
    width: int
    height: int
    baseline: int


def preview_geometry(size, compact, layout):
    spec = layout['preview']['compact' if compact else 'regular']
    fraction = max(0.0, min(1.0, (size - 128) / (512 - 128)))
    return PreviewGeometry(round(spec['minWidth'] + fraction * (spec['maxWidth'] - spec['minWidth'])),
                           round(spec['minHeight'] + fraction * (spec['maxHeight'] - spec['minHeight'])),
                           spec['baseline'])
