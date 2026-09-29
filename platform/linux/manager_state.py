"""Manager state shared by the controller and GTK pages; no toolkit types."""
from __future__ import annotations
from copy import deepcopy
from dataclasses import dataclass, field
from enum import Enum
from typing import Any
from manager_connections import ModelCatalogState


class JobKind(str, Enum):
    RUNTIME = 'runtime'
    STATUS = 'status'
    IMPORT = 'import'
    PREVIEW = 'preview'
    CATALOG = 'catalog'
    RESOURCE = 'resource'


@dataclass
class Route:
    area: str = 'themes'
    screen: str = 'theme'
    tab: str = 'characters'
    library_tab: str = 'characters'
    detail: str | None = None
    selected_actions: dict[str, str] = field(default_factory=dict)


@dataclass
class ThemeDraft:
    value: dict[str, Any]
    store_revision: int = 0
    revision: int = 0
    saved_revision: int = 0
    validation_error: AppError | None = None
    save_error: AppError | None = None

    def edit(self, **changes: Any) -> None:
        self.value.update(changes)
        self.revision += 1
        self.validation_error = None
        self.save_error = None

    def changed(self) -> None:
        self.revision += 1
        self.validation_error = None
        self.save_error = None

    def failed(self, error: AppError, validation: bool) -> None:
        if self.revision == self.saved_revision:
            self.revision += 1
        if validation:
            self.validation_error = error
            self.save_error = None
        else:
            self.save_error = error
            self.validation_error = None

    def saved(self, value: dict[str, Any], store_revision: int | None = None) -> None:
        self.value = value
        self.saved_revision = self.revision
        if store_revision is not None:
            self.store_revision = store_revision
        self.validation_error = None
        self.save_error = None

    @property
    def needs_save(self) -> bool:
        return self.revision != self.saved_revision

    def snapshot(self) -> dict[str, Any]:
        return deepcopy(self.value)


@dataclass
class RuntimeState:
    snapshot: dict[str, Any] | None = None
    disconnected: bool = False
    error_code: str | None = None
    generation: int = 0

    def update(self, snapshot: dict[str, Any] | None) -> None:
        self.snapshot = snapshot
        self.disconnected = False
        self.error_code = None
        self.generation += 1

    def failed(self, code: str) -> None:
        self.disconnected = code in ('runtime.disconnected', 'status.disconnected')
        self.error_code = code
        self.generation += 1


@dataclass
class CommandState:
    active: JobKind | None = None
    queued: list[JobKind] = field(default_factory=list)
    external: set[JobKind] = field(default_factory=set)
    closing: bool = False
    cancel_requested: bool = False
    last_error_code: str | None = None
    visible_message: str | None = None

    @property
    def busy(self) -> bool:
        return self.active is not None or bool(self.queued or self.external)


@dataclass
class AppState:
    route: Route
    draft: ThemeDraft
    runtime: RuntimeState = field(default_factory=RuntimeState)
    commands: CommandState = field(default_factory=CommandState)
    catalog: dict[str, dict] = field(default_factory=dict)
    bubbles: dict[str, str] = field(default_factory=dict)
    plugins: dict[str, dict] = field(default_factory=dict)
    themes: list[dict] = field(default_factory=list)
    session_connection: dict | None = None
    model_catalog: dict[str, dict] = field(default_factory=dict)
    models: ModelCatalogState = field(default_factory=ModelCatalogState)
    settings: dict[str, Any] = field(default_factory=dict)
    last_import: ImportResult | None = None
    closed: bool = False


@dataclass
class AppError(ValueError):
    code: str
    fallback: str
    detail: str = ''

    def __str__(self) -> str:
        return self.fallback


@dataclass(frozen=True)
class ImportResult:
    kind: str
    key: str | None = None
    outcome: str = 'complete'
    issue: AppError | None = None

    @property
    def partial(self) -> bool:
        return self.outcome == 'partial'


def error_from_exception(error: Exception, context: str) -> AppError:
    """Turn service exceptions into stable codes while retaining legacy text."""
    if isinstance(error, AppError):
        return error
    if isinstance(error, (ConnectionError, TimeoutError, BrokenPipeError)):
        code = 'runtime.disconnected' if context in ('runtime', 'status') else context+'.unavailable'
    elif isinstance(error, (FileNotFoundError, KeyError, StopIteration)):
        code = context+'.missing'
    elif isinstance(error, PermissionError):
        code = context+'.permission'
    elif isinstance(error, ValueError):
        code = context+'.invalid'
    else:
        code = context+'.failed'
    return AppError(code, str(error) or 'Requested item was not found.', type(error).__name__)
