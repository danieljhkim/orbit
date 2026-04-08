from __future__ import annotations

from .architecture import GenerateArchitectureComponent
from .base import BaseComponent
from .graph import (
    BuildGraphDirsComponent,
    BuildGraphFilesComponent,
    BuildGraphLeavesComponent,
    PersistGraphComponent,
)
from .hashes import (
    ComputeHashesComponent,
    SaveHashCacheComponent,
    SelectChangedPathsComponent,
)
from .manifest import ManifestComponent
from .scan import ScanRepoComponent
from .summarize import SummarizeFilesComponent

DEFAULT_COMPONENTS = [
    ScanRepoComponent,
    ComputeHashesComponent,
    BuildGraphDirsComponent,
    BuildGraphFilesComponent,
    BuildGraphLeavesComponent,
    PersistGraphComponent,
    ManifestComponent,
    SaveHashCacheComponent,
]

OPTIONAL_COMPONENTS = [
    SelectChangedPathsComponent,
    SummarizeFilesComponent,
    GenerateArchitectureComponent,
]

BUILTIN_COMPONENTS = [
    *DEFAULT_COMPONENTS,
    *OPTIONAL_COMPONENTS,
]

DEFAULT_COMPONENT_NAMES = [component_cls.name for component_cls in DEFAULT_COMPONENTS]

__all__ = [
    "BaseComponent",
    "BuildGraphDirsComponent",
    "BuildGraphFilesComponent",
    "BuildGraphLeavesComponent",
    "BUILTIN_COMPONENTS",
    "ComputeHashesComponent",
    "DEFAULT_COMPONENTS",
    "DEFAULT_COMPONENT_NAMES",
    "GenerateArchitectureComponent",
    "ManifestComponent",
    "OPTIONAL_COMPONENTS",
    "PersistGraphComponent",
    "SaveHashCacheComponent",
    "ScanRepoComponent",
    "SelectChangedPathsComponent",
    "SummarizeFilesComponent",
]
