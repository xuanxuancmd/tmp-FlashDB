#!/usr/bin/env python3
"""
Golden Manifest Builder for FlashDB C→Rust

从 C 源码和测试代码生成黄金清单（强制 tree-sitter-c），用于指导 1:1 翻译迁移
不遗漏源码 API 和测试场景。

安装： pip install tree-sitter tree-sitter-c pyyaml

用法：
    python build_golden_manifest.py --module flashdb --c-root C:/wanglong/temp/FlashDB

输出：
    .opencode/harness/manifests/flashdb.golden.yaml      （源码清单）
    .opencode/harness/manifests/flashdb-test.golden.yaml （测试清单）
"""

import os
import sys
import yaml
import argparse
from pathlib import Path
from datetime import datetime
from typing import Dict, List, Optional

try:
    import tree_sitter_c as tsc
    from tree_sitter import Language, Parser
except ImportError:
    print("Error: tree-sitter-c not installed!")
    print("Install: pip install tree-sitter tree-sitter-c")
    sys.exit(1)

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent.parent
MANIFEST_DIR = PROJECT_ROOT / ".opencode" / "harness" / "manifests"


class CSymbolExtractor:
    """从 C 源码提取符号（强制 tree-sitter-c，AST 级提取）"""

    C_STANDARD_PREFIXES = ('__init', '__attribute', '__builtin')

    def __init__(self):
        self.parser = Parser(Language(tsc.language()))

    def extract_from_file(self, file_path: str) -> Optional[Dict]:
        with open(file_path, 'rb') as f:
            content_bytes = f.read()
        tree = self.parser.parse(content_bytes)
        symbols = {
            'functions': [],
            'structs': [],
            'enums': [],
            'typedefs': [],
            'macros': [],
            'macro_funcs': [],
        }
        self._traverse(tree.root_node, symbols, content_bytes)
        return symbols

    def _traverse(self, node, symbols: Dict, content_bytes: bytes):
        t = node.type

        if t == 'function_definition':
            info = self._extract_function(node, content_bytes)
            if info and not info['name'].startswith(self.C_STANDARD_PREFIXES):
                symbols['functions'].append(info)
            return

        if t == 'struct_specifier':
            name_node = node.child_by_field_name('name')
            if name_node:
                name = self._text(name_node, content_bytes)
                if name and not name.startswith(self.C_STANDARD_PREFIXES):
                    symbols['structs'].append(name)

        elif t == 'enum_specifier':
            name_node = node.child_by_field_name('name')
            if name_node:
                name = self._text(name_node, content_bytes)
                if name:
                    symbols['enums'].append(name)

        elif t == 'type_definition':
            for child in reversed(node.children):
                if child.type == 'type_identifier':
                    name = self._text(child, content_bytes)
                    if name:
                        symbols['typedefs'].append(name)
                    break

        elif t == 'preproc_def':
            for child in node.children:
                if child.type == 'identifier':
                    name = self._text(child, content_bytes)
                    if name and not name.startswith(self.C_STANDARD_PREFIXES):
                        symbols['macros'].append(name)
                    break

        elif t == 'preproc_function_def':
            for child in node.children:
                if child.type == 'identifier':
                    name = self._text(child, content_bytes)
                    if name and not name.startswith(self.C_STANDARD_PREFIXES):
                        symbols['macro_funcs'].append(name)
                    break

        for child in node.children:
            self._traverse(child, symbols, content_bytes)

    def _extract_function(self, node, content_bytes: bytes) -> Optional[Dict]:
        declarator = node.child_by_field_name('declarator')
        if not declarator:
            return None

        func_declarator = self._find_function_declarator(declarator)
        if not func_declarator:
            return None

        name_node = func_declarator.child_by_field_name('declarator')
        if not name_node:
            return None

        name = self._extract_name(name_node, content_bytes)
        if not name:
            return None

        storage = 'extern'
        for child in node.children:
            if child.type == 'storage_class_specifier':
                storage = self._text(child, content_bytes)
                break

        return {
            'name': name,
            'is_public': name.startswith('fdb_') and storage != 'static',
            'is_static': storage == 'static',
        }

    def _find_function_declarator(self, node):
        if node.type == 'function_declarator':
            return node
        for child in node.children:
            result = self._find_function_declarator(child)
            if result:
                return result
        return None

    def _extract_name(self, node, content_bytes: bytes) -> Optional[str]:
        if node.type == 'identifier':
            return self._text(node, content_bytes)
        for child in node.children:
            name = self._extract_name(child, content_bytes)
            if name:
                return name
        return None

    def _text(self, node, content_bytes: bytes) -> str:
        return content_bytes[node.start_byte:node.end_byte].decode('utf-8', errors='replace')


class CTestSymbolExtractor:
    """从 C 测试文件提取测试符号（强制 tree-sitter-c）"""

    def __init__(self):
        self.parser = Parser(Language(tsc.language()))

    def extract_from_file(self, file_path: str) -> Optional[Dict]:
        with open(file_path, 'rb') as f:
            content_bytes = f.read()
        tree = self.parser.parse(content_bytes)
        test_metadata = {
            'test_functions': [],
        }
        self._traverse(tree.root_node, test_metadata, content_bytes)
        return test_metadata

    def _traverse(self, node, test_metadata: Dict, content_bytes: bytes):
        t = node.type

        if t == 'function_definition':
            name = self._extract_function_name(node, content_bytes)
            if name and (name.startswith('test_') or name.endswith('_cb')):
                test_metadata['test_functions'].append(name)

        for child in node.children:
            self._traverse(child, test_metadata, content_bytes)

    def _extract_function_name(self, node, content_bytes: bytes) -> Optional[str]:
        declarator = node.child_by_field_name('declarator')
        if not declarator:
            return None
        func_declarator = self._find_function_declarator(declarator)
        if not func_declarator:
            return None
        name_node = func_declarator.child_by_field_name('declarator')
        if not name_node:
            return None
        return self._extract_name(name_node, content_bytes)

    def _find_function_declarator(self, node):
        if node.type == 'function_declarator':
            return node
        for child in node.children:
            result = self._find_function_declarator(child)
            if result:
                return result
        return None

    def _extract_name(self, node, content_bytes: bytes) -> Optional[str]:
        if node.type == 'identifier':
            return content_bytes[node.start_byte:node.end_byte].decode('utf-8', errors='replace')
        for child in node.children:
            name = self._extract_name(child, content_bytes)
            if name:
                return name
        return None


class GoldenManifestBuilder:
    """构建 FlashDB C→Rust 黄金清单（源码清单 + 测试清单）"""

    SOURCE_DIRS = ['src', 'inc']
    TEST_DIRS = ['tests']

    def build_dual(self, module: str, c_root_input: str):
        c_root_path = Path(c_root_input)
        if c_root_path.is_absolute():
            c_root_abs = c_root_path.resolve()
        else:
            c_root_abs = (PROJECT_ROOT / c_root_input).resolve()

        print(f"=== Building Dual Golden Manifests for {module} ===")
        print(f"C root: {c_root_abs}")

        if not c_root_abs.exists():
            print(f"Error: C root directory not found: {c_root_abs}")
            sys.exit(1)

        c_source_files = self._scan_c_files(c_root_abs, self.SOURCE_DIRS)
        c_test_files = self._scan_c_files(c_root_abs, self.TEST_DIRS)

        print(f"Found {len(c_source_files)} C source files (src/ + inc/)")
        print(f"Found {len(c_test_files)} C test files (tests/)")

        # 提取源码符号
        source_extractor = CSymbolExtractor()
        source_entries = []
        for c_file in c_source_files:
            rel_path = os.path.relpath(c_file, c_root_abs).replace('\\', '/')
            symbols = source_extractor.extract_from_file(c_file)
            if symbols:
                source_entries.append(self._build_source_entry(rel_path, symbols))

        # 提取测试符号
        test_extractor = CTestSymbolExtractor()
        test_entries = []
        for c_file in c_test_files:
            rel_path = os.path.relpath(c_file, c_root_abs).replace('\\', '/')
            test_metadata = test_extractor.extract_from_file(c_file)
            if test_metadata and (test_metadata['test_functions'] or test_metadata['test_constants']):
                test_entries.append(self._build_test_entry(rel_path, test_metadata))

        # 构建源码清单
        source_manifest = {
            'metadata': {
                'module': module,
                'manifest_type': 'source',
                'generated_at': datetime.now().isoformat(),
                'c_root': c_root_input,
                'rust_root': 'src',
                'generator': 'build_golden_manifest.py',
                'note': 'FlashDB C→Rust 1:1 翻译源码黄金清单。'
                        'public API（fdb_* 且非 static）必须 1:1 翻译为 Rust pub fn。'
                        'static 函数翻译为 pub(crate) fn 或私有 fn。'
            },
            'entries': source_entries,
            'summary': self._build_source_summary(source_entries)
        }

        # 构建测试清单
        test_manifest = {
            'metadata': {
                'module': module,
                'manifest_type': 'test',
                'generated_at': datetime.now().isoformat(),
                'c_root': c_root_input,
                'rust_root': 'tests',
                'generator': 'build_golden_manifest.py',
                'note': 'FlashDB C→Rust 测试迁移黄金清单。'
                        '每个 test_* 必须迁移为 Rust integration test（或降级为 UT 并注明理由）。'
                        '每个 *_cb 回调随对应 test_* 一起迁移。'
                        'TEST_* 常量必须 1:1 迁移。'
                        '迁移方式：direct=直接迁移 / degraded=降级为UT / merged=合并 / dropped=丢弃(需注明理由)'
            },
            'test_entries': test_entries,
            'summary': self._build_test_summary(test_entries)
        }

        MANIFEST_DIR.mkdir(parents=True, exist_ok=True)
        source_output = MANIFEST_DIR / f"{module}.golden.yaml"
        test_output = MANIFEST_DIR / f"{module}-test.golden.yaml"

        with open(source_output, 'w', encoding='utf-8') as f:
            yaml.dump(source_manifest, f, default_flow_style=False, allow_unicode=True, sort_keys=False)

        with open(test_output, 'w', encoding='utf-8') as f:
            yaml.dump(test_manifest, f, default_flow_style=False, allow_unicode=True, sort_keys=False)

        print(f"\n=== Dual Manifests Generated ===")
        s = source_manifest['summary']
        print(f"Source manifest: {source_output}")
        print(f"  files={s['file_count']} functions={s['function_count']} public_api={s['public_api_count']} static={s['static_function_count']}")
        print(f"  structs={s['struct_count']} enums={s['enum_count']} typedefs={s['typedef_count']} macros={s['macro_count']} macro_funcs={s['macro_func_count']}")
        t = test_manifest['summary']
        print(f"Test manifest: {test_output}")
        print(f"  test_files={t['test_file_count']} test_functions={t['test_function_count']}")

        return source_manifest, test_manifest

    def _scan_c_files(self, root_abs: Path, sub_dirs: List[str]) -> List[str]:
        c_files = []
        for sub in sub_dirs:
            sub_path = root_abs / sub
            if not sub_path.exists():
                continue
            for root, dirs, files in os.walk(sub_path):
                for f in sorted(files):
                    if f.endswith('.c') or f.endswith('.h'):
                        c_files.append(os.path.join(root, f))
        return sorted(c_files)

    def _build_source_entry(self, rel_path: str, symbols: Dict) -> Dict:
        # 只保留非空字段，函数只保留 name + is_public（is_static 可由 !is_public 推导）
        funcs = [{'name': f['name'], 'public': f['is_public']} for f in symbols['functions']]

        entry = {'c_file': rel_path, 'rust_file': self._c_to_rust_filename(rel_path)}
        if funcs:
            entry['functions'] = funcs
        if symbols['structs']:
            entry['structs'] = symbols['structs']
        if symbols['enums']:
            entry['enums'] = symbols['enums']
        if symbols['typedefs']:
            entry['typedefs'] = symbols['typedefs']
        if symbols['macros']:
            entry['macros'] = symbols['macros']
        if symbols['macro_funcs']:
            entry['macro_funcs'] = symbols['macro_funcs']
        return entry

    def _build_test_entry(self, rel_path: str, test_metadata: Dict) -> Dict:
        entry = {
            'c_test_file': rel_path,
            'rust_test_file': self._c_to_rust_test_filename(rel_path),
        }
        if test_metadata['test_functions']:
            entry['test_functions'] = test_metadata['test_functions']
        return entry

    def _c_to_rust_filename(self, c_rel_path: str) -> str:
        basename = os.path.basename(c_rel_path)
        name_without_ext = os.path.splitext(basename)[0]
        if name_without_ext.startswith('fdb_'):
            rust_name = name_without_ext[4:]
        elif name_without_ext == 'fdb':
            rust_name = 'init'
        else:
            rust_name = name_without_ext
        return f"{rust_name}.rs"

    def _c_to_rust_test_filename(self, c_rel_path: str) -> str:
        basename = os.path.basename(c_rel_path)
        name_without_ext = os.path.splitext(basename)[0]
        if name_without_ext.startswith('fdb_') and name_without_ext.endswith('_tc'):
            module_name = name_without_ext[4:-3]
            return f"tests/c-port/{module_name}_equiv.rs"
        return f"tests/{name_without_ext}.rs"

    def _build_source_summary(self, entries: List[Dict]) -> Dict:
        total = 0
        public = 0
        static = 0
        structs = 0
        enums = 0
        typedefs = 0
        macros = 0
        macro_funcs = 0

        for e in entries:
            funcs = e.get('functions', [])
            total += len(funcs)
            public += sum(1 for f in funcs if f['public'])
            static += sum(1 for f in funcs if not f['public'])
            structs += len(e.get('structs', []))
            enums += len(e.get('enums', []))
            typedefs += len(e.get('typedefs', []))
            macros += len(e.get('macros', []))
            macro_funcs += len(e.get('macro_funcs', []))

        return {
            'file_count': len(entries),
            'function_count': total,
            'public_api_count': public,
            'static_function_count': static,
            'struct_count': structs,
            'enum_count': enums,
            'typedef_count': typedefs,
            'macro_count': macros,
            'macro_func_count': macro_funcs,
        }

    def _build_test_summary(self, test_entries: List[Dict]) -> Dict:
        return {
            'test_file_count': len(test_entries),
            'test_function_count': sum(len(e.get('test_functions', [])) for e in test_entries),
        }


def main():
    parser = argparse.ArgumentParser(
        description='Build Dual Golden Manifests for FlashDB C→Rust (source + test)'
    )
    parser.add_argument('--module', required=True, help='Module name (e.g., flashdb)')
    parser.add_argument(
        '--c-root', required=True,
        help='C root directory, supports relative or absolute path (e.g., C:/wanglong/temp/FlashDB)'
    )
    args = parser.parse_args()
    builder = GoldenManifestBuilder()
    builder.build_dual(args.module, args.c_root)


if __name__ == '__main__':
    main()
