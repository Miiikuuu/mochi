"""Toolkit-free coordination model shared by shell command surfaces."""
from dataclasses import dataclass


@dataclass(frozen=True)
class MutationPresentation:
    import_active: bool = False
    resource_mutation_active: bool = False
    runtime_mutation_active: bool = False
    waiting_for_resource: bool = False
    waiting_for_runtime: bool = False

    @property
    def runtime_allowed(self):
        return not self.resource_mutation_active and not self.runtime_mutation_active

    @property
    def resource_allowed(self):
        return not self.runtime_mutation_active and not self.resource_mutation_active


def project_mutations(runtime_kind=None, resource_task=None):
    runtime = getattr(runtime_kind, 'value', runtime_kind) == 'runtime'
    active = resource_task is not None and resource_task.phase not in ('completed', 'failed', 'cancelled')
    imported = active and resource_task.action == 'import'
    mutation = active and resource_task.action in ('upgrade', 'uninstall', 'finalize', 'cancel_removal')
    queued = bool(active and resource_task.phase == 'queued')
    return MutationPresentation(bool(imported), bool(mutation), runtime,
                                runtime and bool(mutation) and not queued, bool(mutation) and runtime and queued)
