import pytest
from unittest.mock import MagicMock
from orbit_map.service.lineage_pack import render_lineage_pack, _generate_overview
from orbit_map.schemas.graph.nodes import DirNode, FileNode, LeafNode

@pytest.fixture
def mock_service():
    service = MagicMock()
    # Mock navigator
    service.navigator = MagicMock()
    
    # Helper to create mock nodes
    def create_node(node_type, location, node_id=None):
        node = MagicMock()
        node.node_type = node_type
        node.location = location
        node.id = node_id or f"{node_type}:{location}"
        return node

    service.create_node = create_node
    service.selector_for_node.side_effect = lambda n: f"{n.node_type}:{n.location}"
    
    return service

def test_generate_overview_single_file(mock_service):
    file_node = mock_service.create_node("file", "orbit_map/service/graph_context.py")
    mock_service.get_file_context.return_value = MagicMock(
        summary="This file defines GraphContextService.",
        exports=["GraphContextService"]
    )
    
    selector = "file:orbit_map/service/graph_context.py"
    overview = _generate_overview(mock_service, [(selector, file_node)])
    
    assert "File: `file:orbit_map/service/graph_context.py`" in overview
    assert "This file defines GraphContextService." in overview
    assert "Key exports: GraphContextService." in overview

def test_generate_overview_directory(mock_service):
    dir_node = mock_service.create_node("dir", "orbit_map/service")
    dir_node.description = "Service layer for graph operations."
    dir_node.file_children = ["file1", "file2"]
    dir_node.dir_children = []
    
    selector = "dir:orbit_map/service"
    overview = _generate_overview(mock_service, [(selector, dir_node)])
    
    assert "Directory: `dir:orbit_map/service`" in overview
    assert "Service layer for graph operations." in overview
    assert "Contains 2 files and 0 subdirectories." in overview

def test_generate_overview_multiple_with_shared_ancestor(mock_service):
    ancestor = mock_service.create_node("dir", "orbit_map")
    file1 = mock_service.create_node("file", "orbit_map/a.py")
    file2 = mock_service.create_node("file", "orbit_map/b.py")
    
    mock_service.navigator.get_lineage.side_effect = [
        [ancestor, file1],
        [ancestor, file2]
    ]
    
    mock_service.get_file_context.side_effect = [
        MagicMock(summary="Module A", exports=[]),
        MagicMock(summary="Module B", exports=[])
    ]
    
    requested_nodes = [
        ("file:orbit_map/a.py", file1),
        ("file:orbit_map/b.py", file2)
    ]
    
    overview = _generate_overview(mock_service, requested_nodes)
    
    assert "Selection includes 2 nodes under `dir:orbit_map`:" in overview
    assert "- `file:orbit_map/a.py` (file): Module A" in overview
    assert "- `file:orbit_map/b.py` (file): Module B" in overview

def test_generate_overview_multiple_no_shared_ancestor(mock_service):
    # One file in orbit_map, one in orbit_core
    file1 = mock_service.create_node("file", "orbit_map/a.py")
    file2 = mock_service.create_node("file", "orbit_core/b.py")
    
    mock_service.navigator.get_lineage.side_effect = [
        [file1],
        [file2]
    ]
    
    mock_service.get_file_context.side_effect = [
        MagicMock(summary="Module A", exports=[]),
        MagicMock(summary="Module B", exports=[])
    ]
    
    requested_nodes = [
        ("file:orbit_map/a.py", file1),
        ("file:orbit_core/b.py", file2)
    ]
    
    overview = _generate_overview(mock_service, requested_nodes)
    
    assert "Selection includes 2 nodes under root:" in overview
    assert "- `file:orbit_map/a.py` (file): Module A" in overview
    assert "- `file:orbit_core/b.py` (file): Module B" in overview
