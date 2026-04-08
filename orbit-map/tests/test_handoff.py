import orbit_map.service.lineage_pack as lineage_pack_module
import pytest

from orbit_map.schemas import (
    CodebaseGraphV1,
    DirNode,
    FileNode,
    LeafNode,
    SignatureField,
    WorkerHandoffPacket,
)
from orbit_map.schemas.graph.handoff import HandoffConstraint, HandoffRisk
from orbit_map.service.graph_context import GraphContextService


@pytest.fixture
def graph_service():
    root = DirNode(
        id="dir-root",
        name="orbit_map",
        location="orbit_map",
        language="python",
        dir_children=[],
        file_children=["file-graph-context"],
    )
    file_node = FileNode(
        id="file-graph-context",
        name="graph_context.py",
        location="orbit_map/service/graph_context.py",
        language="python",
        parent_id="dir-root",
        extension=".py",
        imports=["typing"],
        exports=["GraphContextService"],
        leaf_children=["leaf-build-handoff-packet"],
    )
    leaf_node = LeafNode(
        id="leaf-build-handoff-packet",
        name="build_handoff_packet",
        location="orbit_map/service/graph_context.py#build_handoff_packet",
        language="python",
        parent_id="file-graph-context",
        kind="function",
        description="Builds a worker handoff packet from validated selectors.",
        input_signature=[SignatureField(name="task_id", annotation="str")],
    )
    graph = CodebaseGraphV1(
        root_dir_id=root.id,
        dirs=[root],
        files=[file_node],
        leaves=[leaf_node],
    )
    return GraphContextService(graph)


def test_worker_handoff_packet_to_markdown_renders_sections(graph_service):
    packet = graph_service.build_handoff_packet(
        task_id="T1",
        task_title="Test handoff",
        task_intent="Explain the worker's scope.",
        root_selectors=["dir:orbit_map"],
        target_selectors=["file:orbit_map/service/graph_context.py"],
        write_selectors=[
            "leaf:orbit_map/service/graph_context.py#build_handoff_packet:function"
        ],
        read_only_selectors=["file:orbit_map/service/graph_context.py"],
        locked_selectors=[],
        expansion_selectors=["dir:orbit_map"],
        risks=[
            HandoffRisk(
                severity="high",
                description="Editing the wrong selector could widen scope.",
                affected_selectors=["file:orbit_map/service/graph_context.py"],
            )
        ],
        constraints=[
            HandoffConstraint(
                description="Do not modify unrelated files.",
                selectors=["file:orbit_map/service/graph_context.py"],
            )
        ],
        knowledge_dir=".orbit/knowledge",
    )

    markdown = packet.to_markdown()

    assert "## Task" in markdown
    assert "## Graph Scope" in markdown
    assert "### Root Nodes" in markdown
    assert "### Write Nodes" in markdown
    assert "## Risks" in markdown
    assert "## Constraints" in markdown
    assert "## Navigation" in markdown
    assert "`dir:orbit_map`" in markdown
    assert "`leaf:orbit_map/service/graph_context.py#build_handoff_packet:function`" in markdown


def test_worker_handoff_packet_to_markdown_respects_budget(graph_service):
    packet = graph_service.build_handoff_packet(
        task_id="T1",
        task_title="Test handoff",
        task_intent="Explain the worker's scope.",
        root_selectors=["dir:orbit_map"],
        target_selectors=["file:orbit_map/service/graph_context.py"],
        write_selectors=[
            "leaf:orbit_map/service/graph_context.py#build_handoff_packet:function"
        ],
    )

    full_markdown = packet.to_markdown(budget=4_000)
    trimmed_markdown = packet.to_markdown(budget=120)

    assert len(trimmed_markdown) < len(full_markdown)
    assert trimmed_markdown.startswith("# Worker Handoff")


def test_build_handoff_packet_resolves_selectors(graph_service):
    packet = graph_service.build_handoff_packet(
        task_id="T1",
        task_title="Test handoff",
        task_intent="Explain the worker's scope.",
        root_selectors=["dir:orbit_map"],
        target_selectors=["file:orbit_map/service/graph_context.py"],
        write_selectors=[
            "leaf:orbit_map/service/graph_context.py#build_handoff_packet:function"
        ],
        read_only_selectors=["file:orbit_map/service/graph_context.py"],
        expansion_selectors=["dir:orbit_map"],
    )

    assert packet.root_nodes[0].role == "root"
    assert packet.target_nodes[0].role == "target"
    assert packet.write_nodes[0].role == "write"
    assert packet.read_only_nodes[0].role == "read_only"
    assert packet.expansion_handles[0].role == "expansion"
    assert packet.lineage_pack_selectors == [
        "dir:orbit_map",
        "file:orbit_map/service/graph_context.py",
        "leaf:orbit_map/service/graph_context.py#build_handoff_packet:function",
    ]


def test_build_handoff_packet_rejects_unknown_selector(graph_service):
    with pytest.raises(ValueError, match="Unknown file selector"):
        graph_service.build_handoff_packet(
            task_id="T1",
            task_intent="Explain the worker's scope.",
            root_selectors=["dir:orbit_map"],
            target_selectors=["file:orbit_map/service/missing.py"],
            write_selectors=[],
        )


def test_worker_handoff_packet_model_dump_round_trip(graph_service):
    packet = graph_service.build_handoff_packet(
        task_id="T1",
        task_title="Test handoff",
        task_intent="Explain the worker's scope.",
        root_selectors=["dir:orbit_map"],
        target_selectors=["file:orbit_map/service/graph_context.py"],
        write_selectors=[
            "leaf:orbit_map/service/graph_context.py#build_handoff_packet:function"
        ],
        knowledge_dir=".orbit/knowledge",
    )

    restored = WorkerHandoffPacket.model_validate(packet.model_dump())

    assert restored == packet


def test_render_lineage_pack_from_handoff_accepts_model_dump(monkeypatch):
    observed = {}

    def fake_render_lineage_pack(knowledge_dir, selectors, **kwargs):
        observed["knowledge_dir"] = knowledge_dir
        observed["selectors"] = selectors
        observed["kwargs"] = kwargs
        return "rendered"

    monkeypatch.setattr(
        lineage_pack_module,
        "render_lineage_pack",
        fake_render_lineage_pack,
    )
    packet = WorkerHandoffPacket(
        task_id="T1",
        task_intent="Explain the worker's scope.",
        knowledge_dir=".orbit/knowledge",
        lineage_pack_selectors=[
            "dir:orbit_map",
            "file:orbit_map/service/graph_context.py",
        ],
    )

    rendered = lineage_pack_module.render_lineage_pack_from_handoff(
        packet.model_dump(),
        budget=321,
    )

    assert rendered == "rendered"
    assert observed == {
        "knowledge_dir": ".orbit/knowledge",
        "selectors": [
            "dir:orbit_map",
            "file:orbit_map/service/graph_context.py",
        ],
        "kwargs": {"budget": 321},
    }
