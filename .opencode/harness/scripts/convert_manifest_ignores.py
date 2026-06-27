#!/usr/bin/env python3
"""Convert golden manifest ignore annotations to graph parity ignore format."""

import sys
from pathlib import Path


def parse_manifest_simple(path: str) -> dict:
    """Simple manifest parser (no PyYAML dependency)."""
    text = Path(path).read_text(encoding='utf-8')
    entries = []
    current_entry = {}
    current_class = {}
    current_section = None
    
    for line in text.split('\n'):
        stripped = line.strip()
        if not stripped or stripped.startswith('#'):
            continue
        
        if stripped.startswith('- java_file:'):
            if current_class and current_class.get('name'):
                current_entry.setdefault('symbols', {}).setdefault('classes', []).append(current_class)
                current_class = {}
            if current_entry:
                entries.append(current_entry)
            current_entry = {'java_file': stripped.split(':', 1)[1].strip()}
            current_section = None
        elif stripped.startswith('expected_rust_file:'):
            current_entry['expected_rust_file'] = stripped.split(':', 1)[1].strip()
        elif stripped == 'classes:':
            if current_class and current_class.get('name'):
                current_entry.setdefault('symbols', {}).setdefault('classes', []).append(current_class)
                current_class = {}
            current_section = 'classes'
        elif stripped == 'interfaces:':
            if current_class and current_class.get('name'):
                current_entry.setdefault('symbols', {}).setdefault('classes', []).append(current_class)
                current_class = {}
            current_section = 'interfaces'
        elif stripped.startswith('- name:') and current_section in ('classes', 'interfaces'):
            if current_class and current_class.get('name'):
                current_entry.setdefault('symbols', {}).setdefault('classes', []).append(current_class)
            current_class = {'name': stripped.split(':', 1)[1].strip()}
        elif stripped.startswith('ignore:') and current_class:
            current_class['ignore'] = stripped.split(':', 1)[1].strip().lower() == 'true'
        elif stripped.startswith('ignore_reason:') and current_class:
            current_class['ignore_reason'] = stripped.split(':', 1)[1].strip().strip('"')
    
    # Flush last items
    if current_class and current_class.get('name'):
        current_entry.setdefault('symbols', {}).setdefault('classes', []).append(current_class)
    if current_entry:
        entries.append(current_entry)
    
    return {'entries': entries}


def _java_file_to_full_class(java_file: str, class_name: str) -> str:
    """Convert java_file path + class name to Java full class path.
    
    Example:
        java_file = "main/java/org/apache/kafka/connect/cli/AbstractConnectCli.java"
        class_name = "AbstractConnectCli"
        → "org.apache.kafka.connect.cli.AbstractConnectCli"
    """
    # Remove common prefixes
    path = java_file
    for prefix in ['main/java/', 'test/java/']:
        if path.startswith(prefix):
            path = path[len(prefix):]
            break
    
    # Remove .java suffix
    if path.endswith('.java'):
        path = path[:-5]
    
    # Convert path separators to dots
    package_path = path.replace('/', '.')
    
    # If the path already ends with the class name, use it directly
    if package_path.endswith('.' + class_name) or package_path == class_name:
        return package_path
    
    # Otherwise, the class might be an inner class - use the file's class as parent
    file_class = package_path.split('.')[-1]
    if file_class != class_name:
        # Inner class: file class is the outer class
        return f"{package_path}.{class_name}"
    
    return package_path


def convert_manifest_ignores(manifest_path: str, output_path: str):
    """Extract ignore annotations from a golden manifest and write to new format."""
    
    manifest = parse_manifest_simple(manifest_path)
    ignores = []
    
    for entry in manifest.get('entries', []):
        java_file = entry.get('java_file', '')
        symbols = entry.get('symbols', {})
        
        for cls in symbols.get('classes', []):
            if isinstance(cls, dict) and cls.get('ignore', False):
                cls_name = cls.get('name', '')
                reason = cls.get('ignore_reason', 'Marked as ignore in golden manifest')
                full_class = _java_file_to_full_class(java_file, cls_name)
                ignores.append({
                    'class': full_class,
                    'reason': reason,
                })
    
    # Write output
    module_name = Path(manifest_path).stem.replace('.golden', '')
    output_lines = [
        f"# {module_name} 模块 ignore 规则",
        "# 从黄金清单自动提取，class 使用 Java 全路径（包名+类名）",
        "#",
        "# 格式说明：",
        "#   class: Java 全路径（必填）—— 匹配图谱中涉及该类的所有边",
        "#   method: Java 方法名（可选）—— 仅匹配该类的指定方法",
        "#   inherits: 父类/接口全路径（可选）—— 仅匹配 implements 边",
        "#   reason: ignore 原因（必填）",
        "",
        "ignores:",
    ]
    
    for ig in ignores:
        output_lines.append(f"  - class: {ig['class']}")
        if 'method' in ig:
            output_lines.append(f"    method: {ig['method']}")
        if 'inherits' in ig:
            output_lines.append(f"    inherits: {ig['inherits']}")
        output_lines.append(f"    reason: \"{ig['reason']}\"")
        output_lines.append("")
    
    Path(output_path).write_text('\n'.join(output_lines), encoding='utf-8')
    print(f"Converted {len(ignores)} ignore rules from {manifest_path}")
    print(f"Output: {output_path}")


if __name__ == '__main__':
    if len(sys.argv) != 3:
        print("Usage: python convert_manifest_ignores.py <manifest.yaml> <output.yaml>")
        sys.exit(1)
    
    convert_manifest_ignores(sys.argv[1], sys.argv[2])
