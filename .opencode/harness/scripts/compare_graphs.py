#!/usr/bin/env python3
"""
Cross-language graph parity comparison engine.

Compares AST-extracted graphs from two languages to detect translation gaps.
Only compares edges relevant to translation completeness:
  - contains: directory/file/class existence
  - method: function existence
  - implements: inheritance/interface implementation
  - calls: call chain consistency

Does NOT compare: references (type usage), imports_from (module imports).

Usage:
    python compare_graphs.py \
        --source-graph java-graph.json \
        --target-graph rust-graph.json \
        --source-language java \
        --target-language rust \
        --filter-crate connect-api \
        --ignores module-ignores.yaml \
        --output candidates.json
"""

import json
import re
import sys
from pathlib import Path
from typing import Optional


# ─── Language-aware naming conventions ────────────────────────────────

LANGUAGE_CONVENTIONS = {
    'java': {'methods': 'camel', 'classes': 'pascal'},
    'rust': {'methods': 'snake', 'classes': 'pascal'},
    'python': {'methods': 'snake', 'classes': 'pascal'},
    'go': {'methods': 'pascal', 'classes': 'pascal'},  # Go uses PascalCase for exported
    'typescript': {'methods': 'camel', 'classes': 'pascal'},
}


def camel_to_snake(name: str) -> str:
    """Convert camelCase to snake_case."""
    s1 = re.sub('(.)([A-Z][a-z]+)', r'\1_\2', name)
    return re.sub('([a-z0-9])([A-Z])', r'\1_\2', s1).lower()


def snake_to_camel(name: str) -> str:
    """Convert snake_case to camelCase."""
    components = name.split('_')
    return components[0] + ''.join(x.title() for x in components[1:])


def pascal_to_snake(name: str) -> str:
    """Convert PascalCase to snake_case."""
    return camel_to_snake(name)


def snake_to_pascal(name: str) -> str:
    """Convert snake_case to PascalCase."""
    return ''.join(x.title() for x in name.split('_'))


def camel_to_pascal(name: str) -> str:
    """Convert camelCase to PascalCase."""
    return name[0].upper() + name[1:] if name else name


def pascal_to_camel(name: str) -> str:
    """Convert PascalCase to camelCase."""
    return name[0].lower() + name[1:] if name else name


def normalize_label(label: str, convention: str) -> str:
    """Normalize a label to a target convention.
    
    Args:
        label: The label to normalize (e.g., "getTaskId", "is_empty")
        convention: Target convention ('camel', 'snake', 'pascal')
    
    Returns:
        Normalized label
    """
    # Strip common prefixes/suffixes
    label = label.lstrip('.')
    label = label.rstrip('()')
    
    # Detect current convention
    if '_' in label:
        current = 'snake'
    elif label[0].isupper() if label else False:
        current = 'pascal'
    else:
        current = 'camel'
    
    # Convert if needed
    if current == convention:
        return label
    elif current == 'camel' and convention == 'snake':
        return camel_to_snake(label)
    elif current == 'snake' and convention == 'camel':
        return snake_to_camel(label)
    elif current == 'pascal' and convention == 'snake':
        return pascal_to_snake(label)
    elif current == 'snake' and convention == 'pascal':
        return snake_to_pascal(label)
    elif current == 'camel' and convention == 'pascal':
        return camel_to_pascal(label)
    elif current == 'pascal' and convention == 'camel':
        return pascal_to_camel(label)
    else:
        return label


def normalize_for_comparison(label: str, source_lang: str, target_lang: str, is_class: bool = False) -> tuple[str, str]:
    """Normalize a label for cross-language comparison.
    
    Args:
        label: The label to normalize
        source_lang: Source language
        target_lang: Target language
        is_class: Whether this is a class/struct name (vs method)
    
    Returns:
        Tuple of (source_normalized, target_normalized)
    """
    source_conv = LANGUAGE_CONVENTIONS.get(source_lang, {})
    target_conv = LANGUAGE_CONVENTIONS.get(target_lang, {})
    
    if is_class:
        source_convention = source_conv.get('classes', 'pascal')
        target_convention = target_conv.get('classes', 'pascal')
    else:
        source_convention = source_conv.get('methods', 'camel')
        target_convention = target_conv.get('methods', 'camel')
    
    # Normalize both to target convention for comparison
    source_norm = normalize_label(label, target_convention)
    target_norm = normalize_label(label, target_convention)
    
    return source_norm, target_norm


# ─── Graph loading ────────────────────────────────────────────────────

def load_graph(path: str) -> dict:
    """Load a graphify AST extraction result."""
    data = json.loads(Path(path).read_text(encoding='utf-8'))
    nodes = {n['id']: n for n in data.get('nodes', [])}
    edges = data.get('edges', [])
    return {'nodes': nodes, 'edges': edges}


def load_ignores(paths: list[str]) -> list[dict]:
    """Load ignore rules from one or more YAML files.
    
    Automatically loads built-in language-ignores from the harness directory.
    
    Ignore format supports 4 types (class uses Java full path):
    
        ignores:
          # Class-level
          - class: org.apache.kafka.connect.converters.BooleanConverter
            reason: "..."
          # Method-level
          - class: org.apache.kafka.connect.cli.AbstractConnectCli
            method: startConnect
            reason: "..."
          # Call-chain-level
          - from_class: org.apache.kafka.connect.runtime.WorkerSourceTask
            from_method: run
            to_class: org.apache.kafka.connect.runtime.WorkerSourceTask
            to_method: validate
            reason: "..."
          # Inheritance-level
          - class: org.apache.kafka.connect.converters.BooleanConverter
            inherits: org.apache.kafka.connect.storage.Converter
            reason: "..."
    
    Args:
        paths: List of paths to ignore YAML files
    
    Returns:
        List of ignore rules
    """
    ignores = []
    
    # Auto-load built-in language ignores
    script_dir = Path(__file__).parent
    language_ignores_dir = script_dir.parent / 'ignores' / 'language-ignores'
    if language_ignores_dir.exists():
        builtin_paths = [str(f) for f in language_ignores_dir.glob('*.yaml')]
        paths = builtin_paths + list(paths)
    
    for path in paths:
        if not Path(path).exists():
            continue
        
        text = Path(path).read_text(encoding='utf-8')
        
        # Simple YAML parser for ignore format
        current_ignore = {}
        for line in text.split('\n'):
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            
            # Detect new entry (starts with "- key:")
            if line.startswith('- '):
                if current_ignore:
                    ignores.append(current_ignore)
                current_ignore = {}
                line = line[2:]  # Remove "- " prefix
            
            # Parse key: value
            if ':' in line and current_ignore is not None:
                key, value = line.split(':', 1)
                key = key.strip()
                value = value.strip().strip('"')
                if key in ('class', 'method', 'reason', 'inherits',
                           'from_class', 'from_method', 'to_class', 'to_method'):
                    current_ignore[key] = value
        
        if current_ignore:
            ignores.append(current_ignore)
    
    return ignores


def _extract_class_name(full_path: str) -> str:
    """Extract the simple class name from a Java full path.
    
    Examples:
        "org.apache.kafka.connect.converters.BooleanConverter" → "BooleanConverter"
        "org.apache.kafka.connect.runtime.Worker.TaskBuilder" → "TaskBuilder"
        "BooleanConverter" → "BooleanConverter"
    """
    # Handle both dot-separated and dollar-separated inner classes
    parts = full_path.replace('$', '.').split('.')
    return parts[-1] if parts else full_path


# ─── Ignore matching ──────────────────────────────────────────────────

def _label_matches_class(label: str, class_full_path: str) -> bool:
    """Check if a graph node label matches a Java class full path.
    
    Extracts the simple class name from the full path and matches against the label.
    
    Examples:
        label="BooleanConverter", path="org.apache...BooleanConverter" → True
        label="boolean_converter", path="org.apache...BooleanConverter" → True
        label="Sink", path="org.apache...LogReporter.Sink" → True
    """
    class_name = _extract_class_name(class_full_path)
    
    label_clean = label.rstrip('()').lstrip('.')
    if class_name.lower() in label_clean.lower():
        return True
    if camel_to_snake(class_name) in label_clean.lower():
        return True
    return False


def _label_matches_method(label: str, method_name: str) -> bool:
    """Check if a graph node label matches a Java method name.
    
    Handles: ".configure()" → "configure", "get_task_id" → "getTaskId"
    """
    label_clean = label.rstrip('()').lstrip('.')
    if method_name == '*':
        return True
    if label_clean.lower() == method_name.lower():
        return True
    if label_clean.lower() == camel_to_snake(method_name):
        return True
    return False


def _node_matches_class(node: dict, class_full_path: str) -> bool:
    """Check if a node matches a Java class full path.
    
    Matches against both the node label and source_file path.
    """
    label = node.get('label', '')
    source_file = node.get('source_file', '')
    
    # Match against label
    if _label_matches_class(label, class_full_path):
        return True
    
    # Match against source_file (class name in file path)
    class_name = _extract_class_name(class_full_path)
    snake_name = camel_to_snake(class_name)
    if class_name.lower() in source_file.lower() or snake_name in source_file.lower():
        return True
    
    return False


def matches_ignore(edge: dict, ignore: dict, source_nodes: dict, target_nodes: dict) -> bool:
    """Check if an edge matches an ignore rule.
    
    Supports 4 types:
    1. Class-level: 'class' field → match edges involving that class
    2. Method-level: 'class' + 'method' → match edges for that method
    3. Call-chain-level: 'from_class/method' + 'to_class/method' → match specific call
    4. Inheritance-level: 'class' + 'inherits' → match implements edges
    
    Args:
        edge: The edge to check
        ignore: The ignore rule
        source_nodes: Source graph nodes dict
        target_nodes: Target graph nodes dict (unused, kept for API compat)
    
    Returns:
        True if the edge matches the ignore rule
    """
    # Get edge endpoints
    source_node = source_nodes.get(edge['source'], {})
    target_node = source_nodes.get(edge['target'], {})
    
    # --- Type 4: Inheritance matching ---
    inherits = ignore.get('inherits', '')
    class_name = ignore.get('class', '')
    
    if inherits and class_name:
        # Match: implements edge where source is class AND target is parent/interface
        if edge['relation'] != 'implements':
            return False
        return (_node_matches_class(source_node, class_name) and 
                _node_matches_class(target_node, inherits))
    
    # --- Type 3: Call-chain matching ---
    from_class = ignore.get('from_class', '')
    from_method = ignore.get('from_method', '')
    to_class = ignore.get('to_class', '')
    to_method = ignore.get('to_method', '')
    
    if from_class and to_class:
        # For calls edges, nodes are methods (label=".run()"),
        # so from_class/to_class match against source_file, not label.
        from_match = True
        if from_class != '*':
            from_file_match = (_node_matches_class(source_node, from_class))
            from_match = from_file_match
        if from_method:
            from_match = from_match and _label_matches_method(
                source_node.get('label', ''), from_method)
        
        to_match = True
        if to_class != '*':
            to_file_match = (_node_matches_class(target_node, to_class))
            to_match = to_file_match
        if to_method:
            to_match = to_match and _label_matches_method(
                target_node.get('label', ''), to_method)
        
        return from_match and to_match
    
    # --- Type 1 & 2: Class-level and Method-level matching ---
    method_name = ignore.get('method', '')
    
    if not class_name:
        return False
    
    # --- Type 2: Method-level matching ---
    if method_name:
        # Match: source involves class AND target is method
        source_in_class = _node_matches_class(source_node, class_name)
        target_is_method = _label_matches_method(target_node.get('label', ''), method_name)
        
        # Also check reverse: source is method, target involves class
        source_is_method = _label_matches_method(source_node.get('label', ''), method_name)
        target_in_class = _node_matches_class(target_node, class_name)
        
        return (source_in_class and target_is_method) or (source_is_method and target_in_class)
    
    # --- Type 1: Class-level matching ---
    return (_node_matches_class(source_node, class_name) or 
            _node_matches_class(target_node, class_name))


# ─── Comparison engine ────────────────────────────────────────────────

def _node_in_scope(node: dict, filter_crate: str | None, filter_dir: str | None) -> bool:
    """Check if a node belongs to the current scope.
    
    Args:
        node: Graph node
        filter_crate: If set, only include nodes whose source_file contains this crate name
        filter_dir: If set, only include nodes whose source_file starts with this directory
    
    Returns:
        True if the node is in scope (or no filter is set)
    """
    if not filter_crate and not filter_dir:
        return True
    
    source_file = node.get('source_file', '')
    if not source_file:
        return True  # Nodes without source_file are always in scope
    
    if filter_crate:
        # Check if source_file path contains the crate name
        # e.g., "connect-rust/connect-api/src/..." contains "connect-api"
        if filter_crate not in source_file:
            return False
    
    if filter_dir:
        # Check if source_file starts with the directory
        # Normalize path separators
        normalized_file = source_file.replace('\\', '/')
        normalized_dir = filter_dir.replace('\\', '/')
        if not normalized_file.startswith(normalized_dir):
            return False
    
    return True


def compare_graphs(
    source_graph: dict,
    target_graph: dict,
    source_lang: str,
    target_lang: str,
    ignores: list[dict],
    filter_crate: str | None = None,
    filter_dir: str | None = None,
) -> dict:
    """Compare two graphs and generate a parity report.
    
    Args:
        source_graph: Source language graph
        target_graph: Target language graph
        source_lang: Source language name
        target_lang: Target language name
        ignores: List of ignore rules
        filter_crate: If set, only compare edges within this crate
        filter_dir: If set, only compare edges within this directory
    
    Returns:
        Comparison report with issues and statistics
    """
    source_nodes = source_graph['nodes']
    target_nodes = target_graph['nodes']
    source_edges = source_graph['edges']
    target_edges = target_graph['edges']
    
    # Only compare edges that matter for translation completeness
    # User requirements: directory, class, function, inheritance, call chain
    RELEVANT_EDGE_TYPES = {'calls', 'method', 'implements', 'contains'}
    
    # Filter edges to only relevant types
    source_edges = [e for e in source_edges if e['relation'] in RELEVANT_EDGE_TYPES]
    target_edges = [e for e in target_edges if e['relation'] in RELEVANT_EDGE_TYPES]
    
    # Index target edges by (relation, source_label, target_label)
    target_edge_index = {}
    for edge in target_edges:
        source_node = target_nodes.get(edge['source'], {})
        target_node = target_nodes.get(edge['target'], {})
        source_label = source_node.get('label', '')
        target_label = target_node.get('label', '')
        
        # Normalize labels for comparison
        source_norm, _ = normalize_for_comparison(source_label, target_lang, target_lang)
        target_norm, _ = normalize_for_comparison(target_label, target_lang, target_lang)
        
        key = (edge['relation'], source_norm.lower(), target_norm.lower())
        target_edge_index[key] = edge
    
    # Compare edges
    issues = []
    ignored_issues = []
    stats = {
        'total_source_edges': len(source_edges),
        'total_target_edges': len(target_edges),
        'matched_edges': 0,
        'missing_edges': 0,
        'ignored_edges': 0,
        'filtered_edges': 0,
    }
    
    for edge in source_edges:
        source_node = source_nodes.get(edge['source'], {})
        target_node = source_nodes.get(edge['target'], {})
        source_label = source_node.get('label', '')
        target_label = target_node.get('label', '')
        
        # Scope filter: skip edges where either endpoint is outside scope
        if filter_crate or filter_dir:
            if not _node_in_scope(source_node, filter_crate, filter_dir) or \
               not _node_in_scope(target_node, filter_crate, filter_dir):
                stats['filtered_edges'] += 1
                continue
        
        # Check if this edge should be ignored
        ignore_match = None
        for ignore in ignores:
            if matches_ignore(edge, ignore, source_nodes, target_nodes):
                ignore_match = ignore
                break
        
        if ignore_match:
            ignored_issues.append({
                'issue_type': f"MISSING_{edge['relation'].upper()}",
                'edge_type': edge['relation'],
                'source_entity': source_label,
                'target_entity': target_label,
                'source_file': source_node.get('source_file', ''),
                'ignore_reason': ignore_match.get('reason', 'Matched ignore rule'),
                'confidence': 'CERTAIN',
            })
            stats['ignored_edges'] += 1
            continue
        
        # Normalize labels for comparison
        source_norm, _ = normalize_for_comparison(source_label, source_lang, target_lang)
        target_norm, _ = normalize_for_comparison(target_label, source_lang, target_lang)
        
        # Look for matching edge in target
        key = (edge['relation'], source_norm.lower(), target_norm.lower())
        
        if key in target_edge_index:
            stats['matched_edges'] += 1
        else:
            # Determine severity based on edge type
            severity_map = {
                'calls': 'HIGH',
                'implements': 'HIGH',
                'contains': 'HIGH',
                'method': 'MEDIUM',
                'imports_from': 'MEDIUM',
                'references': 'LOW',
            }
            severity = severity_map.get(edge['relation'], 'LOW')
            
            issues.append({
                'issue_type': f"MISSING_{edge['relation'].upper()}",
                'severity': severity,
                'edge_type': edge['relation'],
                'source_entity': source_label,
                'target_entity': target_label,
                'source_file': source_node.get('source_file', ''),
                'description': f"Source has {edge['relation']} edge from {source_label} to {target_label}, but target does not",
            })
            stats['missing_edges'] += 1
    
    # Check for extra methods in target (REDUNDANT_METHOD)
    source_methods = set()
    for edge in source_edges:
        if edge['relation'] == 'method':
            target_node = source_nodes.get(edge['target'], {})
            source_methods.add(target_node.get('label', '').lower())
    
    for edge in target_edges:
        if edge['relation'] == 'method':
            target_node = target_nodes.get(edge['target'], {})
            label = target_node.get('label', '')
            
            # Scope filter
            if filter_crate or filter_dir:
                source_node_target = target_nodes.get(edge['source'], {})
                if not _node_in_scope(target_node, filter_crate, filter_dir) or \
                   not _node_in_scope(source_node_target, filter_crate, filter_dir):
                    continue
            
            # Normalize to source convention for comparison
            source_norm, _ = normalize_for_comparison(label, target_lang, source_lang)
            
            if source_norm.lower() not in source_methods:
                # Skip common language-specific methods
                if label.lower() in ('new', 'default', 'drop', 'fmt', 'display', 'clone'):
                    continue
                
                source_node = target_nodes.get(edge['source'], {})
                issues.append({
                    'issue_type': 'EXTRA_METHOD',
                    'severity': 'LOW',
                    'edge_type': 'method',
                    'source_entity': source_node.get('label', ''),
                    'target_entity': label,
                    'source_file': target_node.get('source_file', ''),
                    'description': f"Target has method {label} that does not exist in source",
                })
    
    # Check for extra files in target (REDUNDANT_FILE)
    source_files = set()
    for node in source_nodes.values():
        if 'source_file' in node:
            source_files.add(Path(node['source_file']).stem.lower())
    
    for node in target_nodes.values():
        if 'source_file' in node:
            # Scope filter
            if filter_crate or filter_dir:
                if not _node_in_scope(node, filter_crate, filter_dir):
                    continue
            
            target_file = node['source_file']
            stem = Path(target_file).stem.lower()
            
            # Normalize to source convention
            source_norm, _ = normalize_for_comparison(stem, target_lang, source_lang)
            
            if source_norm not in source_files and stem not in source_files:
                issues.append({
                    'issue_type': 'REDUNDANT_FILE',
                    'severity': 'LOW',
                    'edge_type': 'contains',
                    'source_entity': '',
                    'target_entity': target_file,
                    'source_file': target_file,
                    'description': f"Target has file {target_file} that does not exist in source",
                })
    
    # Sort issues by severity
    severity_order = {'HIGH': 0, 'MEDIUM': 1, 'LOW': 2}
    issues.sort(key=lambda x: severity_order.get(x['severity'], 99))
    
    # Calculate statistics
    total_candidates = len(issues) + len(ignored_issues)
    compared_edges = stats['total_source_edges'] - stats['filtered_edges']
    pass_rate = (stats['matched_edges'] / compared_edges * 100) if compared_edges > 0 else 0
    
    # Count ignore sources
    ignore_audit = {
        'from_language_rules': sum(1 for ig in ignored_issues if 'language' in ig.get('ignore_reason', '').lower()),
        'from_project_rules': sum(1 for ig in ignored_issues if 'project' in ig.get('ignore_reason', '').lower() or 'module' in ig.get('ignore_reason', '').lower()),
    }
    
    # Determine scope
    scope = 'project'
    if filter_crate:
        scope = f'module:{filter_crate}'
    elif filter_dir:
        scope = f'directory:{filter_dir}'
    
    return {
        'issues': issues,
        'ignored_issues': ignored_issues,
        'summary': {
            'scope': scope,
            'total_candidates': total_candidates,
            'real_issues': len(issues),
            'ignored_issues': len(ignored_issues),
            'source_edge_count': stats['total_source_edges'],
            'target_edge_count': stats['total_target_edges'],
            'compared_edges': compared_edges,
            'filtered_edges': stats['filtered_edges'],
            'matched_edges': stats['matched_edges'],
            'missing_edges': stats['missing_edges'],
            'pass_rate': f'{pass_rate:.1f}%',
            'high_severity': sum(1 for i in issues if i['severity'] == 'HIGH'),
            'medium_severity': sum(1 for i in issues if i['severity'] == 'MEDIUM'),
            'low_severity': sum(1 for i in issues if i['severity'] == 'LOW'),
            'ignore_audit': ignore_audit,
        },
    }


# ─── Main ─────────────────────────────────────────────────────────────

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description='Cross-language graph parity comparison')
    parser.add_argument('--source-graph', required=True, help='Source language graph.json')
    parser.add_argument('--target-graph', required=True, help='Target language graph.json')
    parser.add_argument('--source-language', required=True, help='Source language (java, python, etc.)')
    parser.add_argument('--target-language', required=True, help='Target language (rust, go, etc.)')
    parser.add_argument('--ignores', nargs='*', default=[], help='Ignore YAML files (module-specific + built-in)')
    parser.add_argument('--filter-crate', default=None, help='Only compare edges within this crate (scope=module)')
    parser.add_argument('--filter-dir', default=None, help='Only compare edges within this directory (scope=directory)')
    parser.add_argument('--output', default='candidates.json', help='Output candidates file')
    
    args = parser.parse_args()
    
    # Load graphs
    print(f"Loading source graph: {args.source_graph}")
    source_graph = load_graph(args.source_graph)
    print(f"  {len(source_graph['nodes'])} nodes, {len(source_graph['edges'])} edges")
    
    print(f"Loading target graph: {args.target_graph}")
    target_graph = load_graph(args.target_graph)
    print(f"  {len(target_graph['nodes'])} nodes, {len(target_graph['edges'])} edges")
    
    # Load ignores
    ignores = load_ignores(args.ignores)
    print(f"Loaded {len(ignores)} ignore rules")
    
    # Compare
    print(f"\nComparing {args.source_language} → {args.target_language}...")
    if args.filter_crate:
        print(f"  Scope: module ({args.filter_crate})")
    elif args.filter_dir:
        print(f"  Scope: directory ({args.filter_dir})")
    else:
        print(f"  Scope: project (full)")
    
    report = compare_graphs(
        source_graph, target_graph,
        args.source_language, args.target_language,
        ignores,
        filter_crate=args.filter_crate,
        filter_dir=args.filter_dir,
    )
    
    # Output
    output_path = Path(args.output)
    output_path.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    
    # Print summary
    summary = report['summary']
    print(f"\n{'='*60}")
    print(f"  CROSS-LANGUAGE GRAPH PARITY REPORT")
    print(f"{'='*60}")
    print(f"  Scope:            {summary['scope']}")
    print(f"  Total candidates: {summary['total_candidates']}")
    print(f"  Real issues:      {summary['real_issues']}")
    print(f"  Ignored issues:   {summary['ignored_issues']}")
    print(f"  Source edges:     {summary['source_edge_count']}")
    print(f"  Target edges:     {summary['target_edge_count']}")
    print(f"  Compared edges:   {summary['compared_edges']}")
    print(f"  Filtered edges:   {summary['filtered_edges']}")
    print(f"  Matched:          {summary['matched_edges']}")
    print(f"  Missing:          {summary['missing_edges']}")
    print(f"  Pass rate:        {summary['pass_rate']}")
    print(f"{'='*60}")
    print(f"  HIGH severity:    {summary['high_severity']}")
    print(f"  MEDIUM severity:  {summary['medium_severity']}")
    print(f"  LOW severity:     {summary['low_severity']}")
    print(f"{'='*60}")
    
    ia = summary.get('ignore_audit', {})
    print(f"  Ignore audit:")
    print(f"    Language rules:    {ia.get('from_language_rules', 0)}")
    print(f"    Project rules:     {ia.get('from_project_rules', 0)}")
    print(f"{'='*60}")
    
    print(f"\nCandidates written to: {output_path}")


if __name__ == '__main__':
    main()
