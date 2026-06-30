#!/usr/bin/env python3
"""
Manifest Parity Verifier for FlashDB C→Rust (双清单自动校验)
校验 Rust 实现与黄金清单的一致性（强制 tree-sitter-rust）

安装： pip install tree-sitter tree-sitter-rust pyyaml

重要约束：
- 强制依赖 tree-sitter，无 fallback
- 自动校验源码清单和测试清单
- 输出路径固定：{module}-parity.json 和 {module}-test-parity.json

用法：
    python verify_manifest_parity.py --module flashdb --rust-root .
    python verify_manifest_parity.py --module flashdb --rust-root . --ignores .opencode/harness/ignores/flashdb-ignores.yaml

输出：
    .opencode/harness/evidence/flashdb-parity.json      （源码校验）
    .opencode/harness/evidence/flashdb-test-parity.json （测试校验）
"""

import os
import re
import sys
import json
import yaml
import argparse
from pathlib import Path
from datetime import datetime
from typing import Dict, List, Set, Tuple, Optional

try:
    import tree_sitter_rust as tsrust
    from tree_sitter import Language, Parser
except ImportError:
    print("Error: tree-sitter-rust not installed!")
    print("Install: pip install tree-sitter tree-sitter-rust")
    sys.exit(1)

# 项目根目录（脚本在 .opencode/harness/scripts/ 下，向上三级）
PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent.parent
MANIFEST_DIR = PROJECT_ROOT / ".opencode" / "harness" / "manifests"
EVIDENCE_DIR = PROJECT_ROOT / ".opencode" / "harness" / "evidence"


# ---------------------------------------------------------------------------
# ignore 规则辅助函数
# ---------------------------------------------------------------------------

def _is_function_ignored(func_name: str, ignored_functions: set) -> bool:
    """检查函数是否被忽略（支持 * 通配符）"""
    if '*' in ignored_functions:
        return True
    return func_name in ignored_functions


def _is_test_ignored(test_name: str, ignored_tests: set) -> bool:
    """检查测试函数是否被忽略（支持 * 通配符）"""
    if '*' in ignored_tests:
        return True
    return test_name in ignored_tests


# ---------------------------------------------------------------------------
# Rust 符号提取器
# ---------------------------------------------------------------------------

class RustSymbolExtractor:
    """从 Rust 源码/测试文件提取符号（强制 tree-sitter）"""

    # FlashDB Rust 约定的辅助方法名，比对时排除（不对应 C 函数）
    RUST_AUXILIARY_NAMES = {'new', 'default', 'fmt', 'drop', 'clone', 'eq', 'hash', 'partial_cmp', 'cmp'}

    def __init__(self):
        self.parser = Parser(Language(tsrust.language()))

    def extract_from_file(self, file_path: str) -> Optional[Dict]:
        try:
            with open(file_path, 'rb') as f:
                content_bytes = f.read()
            return self._extract(content_bytes)
        except Exception as e:
            print(f"Error extracting from {file_path}: {e}", file=sys.stderr)
            return None

    def _extract(self, content_bytes: bytes) -> Dict:
        tree = self.parser.parse(content_bytes)
        root = tree.root_node
        symbols = {
            'functions': [],          # 独立函数（不在 impl 块内）[{name, is_pub}]
            'impl_functions': {},     # 按 struct 分组 {struct_name: [func_name]}
            'structs': [],            # [{name}]
            'enums': [],              # [{name}]
            'test_functions': [],     # [{name, assertion_count, fake_assertion_risk}]
            'c_comments': [],         # // c: xxx.c:LINE 注释收集（用于函数溯源）
        }
        self._traverse(root, symbols, content_bytes)
        return symbols

    def _traverse(self, node, symbols: Dict, content_bytes: bytes, current_impl: Optional[str] = None):
        t = node.type

        if t == 'struct_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                symbols['structs'].append({'name': self._text(name_node, content_bytes)})

        elif t == 'enum_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                symbols['enums'].append({'name': self._text(name_node, content_bytes)})

        elif t == 'impl_item':
            impl_target = None
            type_identifiers = []
            for child in node.children:
                if child.type == 'type_identifier':
                    type_identifiers.append(self._text(child, content_bytes))
            if len(type_identifiers) == 1:
                impl_target = type_identifiers[0]
            elif len(type_identifiers) >= 2:
                impl_target = type_identifiers[1]
            for child in node.children:
                self._traverse(child, symbols, content_bytes, current_impl=impl_target)
            return

        elif t == 'function_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                func_name = self._text(name_node, content_bytes)
                is_pub = self._is_pub(node, content_bytes)

                if current_impl:
                    symbols['impl_functions'].setdefault(current_impl, []).append(func_name)
                else:
                    symbols['functions'].append({'name': func_name, 'is_pub': is_pub})

                # 检查是否为测试函数
                if self._is_test_function(node, content_bytes):
                    func_text = self._text(node, content_bytes)
                    symbols['test_functions'].append({
                        'name': func_name,
                        'assertion_count': self._count_assertions(func_text),
                        'fake_assertion_risk': self._detect_fake_assertion(func_text),
                    })

        elif t == 'function_signature_item':
            for child in node.children:
                if child.type in ('identifier', 'field_identifier'):
                    func_name = self._text(child, content_bytes)
                    if current_impl:
                        symbols['impl_functions'].setdefault(current_impl, []).append(func_name)
                    else:
                        symbols['functions'].append({'name': func_name, 'is_pub': False})
                    break

        # 收集 // c: xxx.c:LINE 注释
        elif t == 'line_comment':
            comment_text = self._text(node, content_bytes)
            if re.search(r'//\s*c:\s*\w+\.c:', comment_text):
                symbols['c_comments'].append(comment_text.strip())

        for child in node.children:
            self._traverse(child, symbols, content_bytes, current_impl=current_impl)

    def _is_pub(self, node, content_bytes: bytes) -> bool:
        parent = node.parent
        if parent:
            for child in parent.children:
                if child == node:
                    break
                if child.type == 'visibility_modifier':
                    return True
        return False

    def _is_test_function(self, node, content_bytes: bytes) -> bool:
        parent = node.parent
        if parent:
            for child in parent.children:
                if child == node:
                    break
                if child.type == 'attribute_item':
                    for attr_child in child.children:
                        if attr_child.type == 'attribute':
                            if self._contains_test_identifier(attr_child, content_bytes):
                                return True
        return False

    def _contains_test_identifier(self, node, content_bytes: bytes) -> bool:
        if node.type == 'identifier':
            if self._text(node, content_bytes) == 'test':
                return True
        for child in node.children:
            if self._contains_test_identifier(child, content_bytes):
                return True
        return False

    def _count_assertions(self, func_text: str) -> int:
        patterns = [
            r'assert(?:_eq|_ne|_matches)?!',
            r'should_panic',
            r'\.is_err\s*\(\)',
            r'\.is_ok\s*\(\)',
            r'\.is_some\s*\(\)',
            r'\.is_none\s*\(\)',
            r'panic!',
            r'unreachable!',
            r'assert(?:_error|_no_error|_success|_failure|_valid|_invalid|_equals|_true|_false)\s*\(',
        ]
        count = 0
        for pattern in patterns:
            count += len(re.findall(pattern, func_text))
        return count

    def _detect_fake_assertion(self, func_text: str) -> bool:
        if re.search(r'assert!\s*\(\s*(?:true|false)\s*\)', func_text):
            return True
        if re.search(r'assert_eq!\s*\((\w+),\s*\1\s*\)', func_text):
            return True
        if 'println!' in func_text and self._count_assertions(func_text) == 0:
            return True
        if len(func_text.strip()) < 50:
            return True
        return False

    def _text(self, node, content_bytes: bytes) -> str:
        return content_bytes[node.start_byte:node.end_byte].decode('utf-8', errors='replace')


# ---------------------------------------------------------------------------
# 名称转换：C 函数名 ↔ Rust 函数名
# ---------------------------------------------------------------------------

def c_to_rust_fn_name(c_name: str) -> str:
    """C 函数名转 Rust 函数名

    FlashDB 约定：fdb_ 前缀保留，下划线风格已一致，无需转换。
    fdb_kv_get → fdb_kv_get（Rust 中作为 impl 方法去掉 fdb_ 前缀和 db 参数）
    fdb_kv_set → kv_set（impl 方法）

    匹配策略：
    1. 完整名匹配（fdb_kv_get）
    2. 去 fdb_ 前缀（kv_get）
    3. 去 fdb_<module>_ 前缀（kvdb → 去 fdb_kvdb_，tsdb → 去 fdb_tsdb_，tsl → 去 fdb_tsl_）
    """
    return c_name


def c_fn_candidates(c_name: str) -> List[str]:
    """生成 C 函数名在 Rust 中可能的对应名称候选

    fdb_kv_get → ['fdb_kv_get', 'kv_get']
    fdb_kvdb_init → ['fdb_kvdb_init', 'kvdb_init', 'init']
    fdb_tsl_append → ['fdb_tsl_append', 'tsl_append', 'append']
    fdb_tsdb_init → ['fdb_tsdb_init', 'tsdb_init', 'init']
    _fdb_flash_read → ['_fdb_flash_read', 'flash_read']
    """
    candidates = [c_name]

    # 去掉 fdb_ 前缀
    if c_name.startswith('fdb_'):
        without_fdb = c_name[4:]
        candidates.append(without_fdb)

        # 去掉模块前缀（kvdb_/tsdb_/tsl_/kv_）
        for module_prefix in ['kvdb_', 'tsdb_', 'tsl_', 'kv_']:
            if without_fdb.startswith(module_prefix):
                candidates.append(without_fdb[len(module_prefix):])
                break

    return candidates


def c_test_to_rust_test_name(c_test_name: str) -> str:
    """C 测试函数名转 Rust 测试函数名

    约定：C 的 test_fdb_xxx → Rust 的 test_c_equiv_xxx 或 c_equiv_xxx
    但实际迁移可能用不同命名，所以用模糊匹配——保留原始名作候选，
    并生成常见转换形式。
    """
    candidates = [c_test_name]

    # test_fdb_kvdb_init → c_equiv_kvdb_init, test_c_equiv_kvdb_init
    if c_test_name.startswith('test_fdb_'):
        rest = c_test_name[9:]  # 去 test_fdb_
        candidates.append(f'c_equiv_{rest}')
        candidates.append(f'test_c_equiv_{rest}')

    return candidates


# ---------------------------------------------------------------------------
# 校验器
# ---------------------------------------------------------------------------

class ManifestParityVerifier:
    """校验 Rust 实现与 C 黄金清单的一致性"""

    def load_ignores(self, ignore_file_path: str) -> Tuple[set, set]:
        """加载外部 ignore YAML 文件

        FlashDB 的 ignore 规则比 Java 简单，只有两类：
        - function 级：忽略指定 C 函数
        - test 级：忽略指定 C 测试

        Returns:
            (ignored_functions, ignored_tests) 元组
        """
        ignored_functions: Set[str] = set()
        ignored_tests: Set[str] = set()

        ignore_path = Path(ignore_file_path)
        if not ignore_path.is_absolute():
            ignore_path = PROJECT_ROOT / ignore_path

        if not ignore_path.exists():
            print(f"Warning: Ignore file not found: {ignore_path}")
            return ignored_functions, ignored_tests

        with open(ignore_path, 'r', encoding='utf-8') as f:
            data = yaml.safe_load(f)

        if not data:
            return ignored_functions, ignored_tests

        for entry in data.get('ignores', []):
            if not isinstance(entry, dict):
                continue
            kind = entry.get('kind', '')
            name = entry.get('name', '')
            if not name:
                continue
            if kind == 'function':
                ignored_functions.add(name)
            elif kind == 'test':
                ignored_tests.add(name)

        print(f"Loaded ignores: {len(ignored_functions)} functions, {len(ignored_tests)} tests")
        return ignored_functions, ignored_tests

    def verify_dual(self, module: str, rust_root_rel: str,
                    ignored_functions: Set[str] = None,
                    ignored_tests: Set[str] = None):
        if ignored_functions is None:
            ignored_functions = set()
        if ignored_tests is None:
            ignored_tests = set()

        rust_root_abs = (PROJECT_ROOT / rust_root_rel).resolve()

        print(f"=== Verifying Dual Manifest Parity for {module} ===")
        print(f"Project root: {PROJECT_ROOT}")
        print(f"Rust root: {rust_root_abs}")

        source_manifest_path = MANIFEST_DIR / f"{module}.golden.yaml"
        test_manifest_path = MANIFEST_DIR / f"{module}-test.golden.yaml"

        if source_manifest_path.exists():
            source_report = self._verify_source_manifest(
                source_manifest_path, rust_root_abs / "src", ignored_functions
            )
        else:
            source_report = {'summary': {'overall_pass': False, 'reason': 'Source manifest not found'}}

        if test_manifest_path.exists():
            test_report = self._verify_test_manifest(
                test_manifest_path, rust_root_abs / "tests", ignored_tests
            )
        else:
            test_report = {'summary': {'overall_pass': False, 'reason': 'Test manifest not found'}}

        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        source_output = EVIDENCE_DIR / f"{module}-parity.json"
        test_output = EVIDENCE_DIR / f"{module}-test-parity.json"

        with open(source_output, 'w', encoding='utf-8') as f:
            json.dump(source_report, f, indent=2, ensure_ascii=False)
        with open(test_output, 'w', encoding='utf-8') as f:
            json.dump(test_report, f, indent=2, ensure_ascii=False)

        print(f"\n=== Dual Verification Reports Generated ===")
        print(f"Source parity: {source_output}")
        print(f"  Issues found: {source_report['summary'].get('issues_found', 0)}")
        print(f"  Overall pass: {source_report['summary'].get('overall_pass', False)}")
        print(f"Test parity: {test_output}")
        print(f"  Issues found: {test_report['summary'].get('issues_found', 0)}")
        print(f"  Overall pass: {test_report['summary'].get('overall_pass', False)}")

        return source_report, test_report

    # -----------------------------------------------------------------
    # 源码清单校验
    # -----------------------------------------------------------------

    def _verify_source_manifest(self, manifest_path: Path, rust_src_root: Path,
                                ignored_functions: Set[str]) -> Dict:
        print(f"\n--- Verifying Source Manifest ---")

        with open(manifest_path, 'r', encoding='utf-8') as f:
            manifest = yaml.safe_load(f)

        if not rust_src_root.exists():
            return {
                'metadata': {'manifest_type': 'source', 'verified_at': datetime.now().isoformat()},
                'summary': {'overall_pass': False, 'reason': f'Rust src root not found: {rust_src_root}'}
            }

        # 扫描 Rust 源码文件（排除 tests 目录）
        rust_files = self._scan_rust_files(rust_src_root, exclude_tests=True)
        print(f"Found {len(rust_files)} Rust source files")

        # 提取 Rust 符号
        extractor = RustSymbolExtractor()
        rust_symbols = {}
        for rust_file in rust_files:
            symbols = extractor.extract_from_file(rust_file)
            if symbols:
                rust_symbols[rust_file] = symbols

        # 建立 rust_file 相对路径映射（相对 src/）
        rust_file_paths = {}
        for rf in rust_files:
            path_obj = Path(rf)
            parts = path_obj.parts
            if 'src' in parts:
                src_idx = parts.index('src')
                rel_parts = parts[src_idx + 1:]
                rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                rust_file_paths[rel_fp] = rf

        # 收集所有 Rust 函数名（含独立函数 + impl 函数），用于全局匹配
        all_rust_fn_names: Set[str] = set()
        # rust_file → 所有函数名集合（独立 + impl）
        rust_file_fn_map: Dict[str, Set[str]] = {}
        for rf, symbols in rust_symbols.items():
            fn_names = set()
            for f in symbols.get('functions', []):
                fn_names.add(f['name'])
            for impl_fns in symbols.get('impl_functions', {}).values():
                fn_names.update(impl_fns)
            all_rust_fn_names.update(fn_names)
            rel_fp = None
            parts = Path(rf).parts
            if 'src' in parts:
                src_idx = parts.index('src')
                rel_fp = str(Path(*parts[src_idx + 1:])).replace('\\', '/')
            rust_file_fn_map[rel_fp or rf] = fn_names

        issues = []
        expected_files = set()
        expected_functions = set()

        for entry in manifest.get('entries', []):
            c_file = entry.get('c_file', '')
            expected_rust_file = entry.get('rust_file', '')
            expected_files.add(expected_rust_file)

            functions = entry.get('functions', [])

            # 问题类型1: Rust 文件缺失
            rust_file_abs = rust_file_paths.get(expected_rust_file)
            if not rust_file_abs:
                # 文件不存在，但函数可能被迁移到其他文件（如 fdb.c → init.rs 或 lib.rs）
                # 检查该 entry 的所有函数是否在全局存在
                for func in functions:
                    c_fn = func['name']
                    if _is_function_ignored(c_fn, ignored_functions):
                        continue
                    expected_functions.add(c_fn)
                    candidates = c_fn_candidates(c_fn)
                    if not any(c in all_rust_fn_names for c in candidates):
                        issues.append({
                            'c_file': c_file,
                            'c_function': c_fn,
                            'expected_rust_file': expected_rust_file,
                            'issue_type': 'FILE_AND_FUNCTION_MISSING',
                            'reason': f'Rust 文件缺失且函数未找到: {c_fn}',
                            'severity': 'HIGH',
                            'is_public': func.get('public', False),
                        })
                continue

            # 文件存在，获取该文件的所有函数名
            file_fn_names = rust_file_fn_map.get(expected_rust_file, set())

            for func in functions:
                c_fn = func['name']
                if _is_function_ignored(c_fn, ignored_functions):
                    continue
                expected_functions.add(c_fn)

                candidates = c_fn_candidates(c_fn)
                matched = any(c in file_fn_names for c in candidates)

                if not matched:
                    # 函数可能在其他文件中（如 init 函数在 lib.rs 而非 init.rs）
                    global_matched = any(c in all_rust_fn_names for c in candidates)
                    if global_matched:
                        # 函数存在但不在预期文件中——降级为 LOW
                        issues.append({
                            'c_file': c_file,
                            'c_function': c_fn,
                            'expected_rust_file': expected_rust_file,
                            'issue_type': 'FUNCTION_IN_WRONG_FILE',
                            'reason': f'函数 {c_fn} 存在但不在预期文件 {expected_rust_file}',
                            'severity': 'LOW',
                            'is_public': func.get('public', False),
                        })
                    else:
                        issues.append({
                            'c_file': c_file,
                            'c_function': c_fn,
                            'expected_rust_file': expected_rust_file,
                            'issue_type': 'FUNCTION_MISSING',
                            'reason': f'Rust 缺失函数: {c_fn}',
                            'severity': 'HIGH' if func.get('public', False) else 'MEDIUM',
                            'is_public': func.get('public', False),
                        })

        # 问题类型2: 冗余文件（Rust 中有但清单中没有）
        excluded_filenames = {'lib.rs', 'mod.rs', 'main.rs', 'build.rs'}
        redundant_files = []
        for rf in set(rust_file_paths.keys()) - expected_files:
            filename = rf.split('/')[-1]
            if filename not in excluded_filenames:
                redundant_files.append(rf)

        for rf in sorted(redundant_files):
            fn_count = len(rust_file_fn_map.get(rf, set()))
            issues.append({
                'rust_file': rf,
                'issue_type': 'REDUNDANT_FILE',
                'reason': 'Rust 文件冗余（黄金清单中无对应 C 文件）',
                'severity': 'LOW',
                'rust_func_count': fn_count,
            })

        # 构建报告
        total_entries = len(manifest.get('entries', []))
        total_functions = sum(len(e.get('functions', [])) for e in manifest.get('entries', []))
        total_public = sum(
            1 for e in manifest.get('entries', [])
            for f in e.get('functions', []) if f.get('public', False)
        )
        issues_count = len(issues)

        return {
            'metadata': {
                'manifest_type': 'source',
                'verified_at': datetime.now().isoformat(),
                'rust_root': str(rust_src_root),
                'tree_sitter_used': True,
            },
            'summary': {
                'total_c_files': total_entries,
                'total_c_functions': total_functions,
                'total_public_api': total_public,
                'total_rust_files': len(rust_files),
                'total_redundant_files': len(redundant_files),
                'issues_found': issues_count,
                'overall_pass': issues_count == 0,
            },
            'issues': issues,
            'issue_breakdown': self._build_source_issue_breakdown(issues),
        }

    def _build_source_issue_breakdown(self, issues: List[Dict]) -> Dict:
        issue_types = [
            'FILE_AND_FUNCTION_MISSING',
            'FUNCTION_MISSING',
            'FUNCTION_IN_WRONG_FILE',
            'REDUNDANT_FILE',
        ]
        breakdown = {}
        for it in issue_types:
            count = len([i for i in issues if i['issue_type'] == it])
            if count > 0:
                breakdown[it] = count
        return breakdown

    # -----------------------------------------------------------------
    # 测试清单校验
    # -----------------------------------------------------------------

    def _verify_test_manifest(self, manifest_path: Path, rust_tests_root: Path,
                              ignored_tests: Set[str]) -> Dict:
        print(f"\n--- Verifying Test Manifest ---")

        with open(manifest_path, 'r', encoding='utf-8') as f:
            manifest = yaml.safe_load(f)

        # 扫描 Rust 测试文件
        rust_test_files = []
        if rust_tests_root.exists():
            for root, dirs, files in os.walk(rust_tests_root):
                dirs[:] = [d for d in dirs if 'target' not in d and 'features' not in d and 'bdd' not in d]
                for file in files:
                    if file.endswith('.rs') and not file.startswith('mod.rs'):
                        rust_test_files.append(os.path.join(root, file))

        print(f"Found {len(rust_test_files)} Rust test files")

        # 提取 Rust 测试符号
        extractor = RustSymbolExtractor()
        rust_test_symbols = {}
        for rust_file in rust_test_files:
            symbols = extractor.extract_from_file(rust_file)
            if symbols:
                rust_test_symbols[rust_file] = symbols

        # 建立 rust_file 相对路径映射（相对 tests/）
        rust_file_paths = {}
        for rf in rust_test_files:
            path_obj = Path(rf)
            parts = path_obj.parts
            if 'tests' in parts:
                tests_idx = parts.index('tests')
                rel_parts = parts[tests_idx + 1:]
                rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                rust_file_paths[rel_fp] = rf

        issues = []
        expected_test_files = set()
        total_expected_test_methods = 0

        for entry in manifest.get('test_entries', []):
            c_test_file = entry.get('c_test_file', '')
            expected_rust_test_file = entry.get('rust_test_file', '')

            # 提取相对路径（去 tests/ 前缀）
            if expected_rust_test_file.startswith('tests/'):
                rust_test_file_key = expected_rust_test_file[6:]
            elif expected_rust_test_file.startswith('tests\\'):
                rust_test_file_key = expected_rust_test_file[6:].replace('\\', '/')
            else:
                rust_test_file_key = expected_rust_test_file
            expected_test_files.add(rust_test_file_key)

            test_functions = entry.get('test_functions', [])
            total_expected_test_methods += len(test_functions)

            # 问题类型1: 测试文件缺失
            rust_file_abs = rust_file_paths.get(rust_test_file_key)
            if not rust_file_abs:
                # 文件不存在，检查测试是否迁移到其他文件
                # 收集所有 Rust 测试函数名
                all_rust_test_names = set()
                for symbols in rust_test_symbols.values():
                    for tf in symbols.get('test_functions', []):
                        all_rust_test_names.add(tf['name'])

                for c_test in test_functions:
                    if _is_test_ignored(c_test, ignored_tests):
                        continue
                    candidates = c_test_to_rust_test_name(c_test)
                    if not any(c in all_rust_test_names for c in candidates):
                        issues.append({
                            'c_test_file': c_test_file,
                            'c_test_function': c_test,
                            'expected_rust_test_file': expected_rust_test_file,
                            'issue_type': 'TEST_FILE_AND_FUNCTION_MISSING',
                            'reason': f'Rust 测试文件缺失且测试函数未找到: {c_test}',
                            'severity': 'HIGH',
                        })
                continue

            # 文件存在，获取该文件的测试函数
            symbols = rust_test_symbols.get(rust_file_abs, {})
            rust_test_fns = symbols.get('test_functions', [])
            rust_test_names = set(tf['name'] for tf in rust_test_fns)

            # 问题类型2: 测试方法缺失
            missing_tests = []
            for c_test in test_functions:
                if _is_test_ignored(c_test, ignored_tests):
                    continue
                candidates = c_test_to_rust_test_name(c_test)
                matched = any(c in rust_test_names for c in candidates)
                if not matched:
                    missing_tests.append(c_test)

            if missing_tests:
                issues.append({
                    'c_test_file': c_test_file,
                    'expected_rust_test_file': expected_rust_test_file,
                    'issue_type': 'TEST_METHOD_MISSING',
                    'reason': f'缺失测试方法: {", ".join(missing_tests[:10])}' + ('...' if len(missing_tests) > 10 else ''),
                    'severity': 'HIGH',
                    'missing_tests': missing_tests,
                })

            # 问题类型3: 测试数量偏差（>30% 报警）
            expected_count = len([t for t in test_functions if not _is_test_ignored(t, ignored_tests)])
            actual_count = len(rust_test_fns)
            if expected_count > 0:
                deviation = abs(expected_count - actual_count) / expected_count
                if deviation > 0.3:
                    issues.append({
                        'c_test_file': c_test_file,
                        'expected_rust_test_file': expected_rust_test_file,
                        'issue_type': 'TEST_COUNT_MISMATCH',
                        'reason': f'测试方法数量偏差: C={expected_count}, Rust={actual_count}',
                        'severity': 'HIGH',
                        'deviation': f'{deviation:.2%}',
                    })

            # 逐个测试检查断言质量
            rust_fn_map = {tf['name']: tf for tf in rust_test_fns}
            for c_test in test_functions:
                if _is_test_ignored(c_test, ignored_tests):
                    continue
                candidates = c_test_to_rust_test_name(c_test)
                matching = None
                for c in candidates:
                    if c in rust_fn_map:
                        matching = rust_fn_map[c]
                        break

                if not matching:
                    continue

                # 问题类型4: 无断言
                if matching.get('assertion_count', 0) == 0:
                    issues.append({
                        'c_test_file': c_test_file,
                        'c_test_function': c_test,
                        'expected_rust_test_file': expected_rust_test_file,
                        'rust_test_function': matching['name'],
                        'issue_type': 'RUST_TEST_NO_ASSERTION',
                        'reason': 'Rust 测试方法无断言',
                        'severity': 'HIGH',
                    })

                # 问题类型5: 虚假断言
                if matching.get('fake_assertion_risk', False):
                    issues.append({
                        'c_test_file': c_test_file,
                        'c_test_function': c_test,
                        'expected_rust_test_file': expected_rust_test_file,
                        'rust_test_function': matching['name'],
                        'issue_type': 'RUST_FAKE_ASSERTION',
                        'reason': 'Rust 测试存在虚假断言风险',
                        'severity': 'HIGH',
                    })

        # 问题类型6: 冗余测试文件
        excluded_filenames = {'mod.rs', 'lib.rs', 'bdd.rs', 'cucumber.rs'}
        excluded_dirs = {'bdd', 'cucumber', 'features', 'steps'}
        redundant_test_files = []
        for rf in set(rust_file_paths.keys()) - expected_test_files:
            filename = rf.split('/')[-1]
            dir_path = '/'.join(rf.split('/')[:-1]) if '/' in rf else ''
            if filename not in excluded_filenames and dir_path not in excluded_dirs:
                redundant_test_files.append(rf)

        for rf in sorted(redundant_test_files):
            symbols = rust_test_symbols.get(rust_file_paths[rf], {})
            test_count = len(symbols.get('test_functions', []))
            issues.append({
                'rust_test_file': rf,
                'issue_type': 'REDUNDANT_TEST_FILE',
                'reason': 'Rust 测试文件冗余（黄金清单中无对应 C 测试文件）',
                'severity': 'LOW',
                'rust_test_func_count': test_count,
            })

        # 构建报告
        total_test_entries = len(manifest.get('test_entries', []))
        issues_count = len(issues)

        return {
            'metadata': {
                'manifest_type': 'test',
                'verified_at': datetime.now().isoformat(),
                'rust_tests_root': str(rust_tests_root),
                'tree_sitter_used': True,
            },
            'summary': {
                'total_c_test_files': total_test_entries,
                'total_expected_test_methods': total_expected_test_methods,
                'total_rust_test_files': len(rust_test_files),
                'total_redundant_test_files': len(redundant_test_files),
                'issues_found': issues_count,
                'overall_pass': issues_count == 0,
            },
            'issues': issues,
            'issue_breakdown': self._build_test_issue_breakdown(issues),
        }

    def _build_test_issue_breakdown(self, issues: List[Dict]) -> Dict:
        issue_types = [
            'TEST_FILE_AND_FUNCTION_MISSING',
            'TEST_METHOD_MISSING',
            'TEST_COUNT_MISMATCH',
            'RUST_TEST_NO_ASSERTION',
            'RUST_FAKE_ASSERTION',
            'REDUNDANT_TEST_FILE',
        ]
        breakdown = {}
        for it in issue_types:
            count = len([i for i in issues if i['issue_type'] == it])
            if count > 0:
                breakdown[it] = count
        return breakdown

    # -----------------------------------------------------------------
    # 辅助
    # -----------------------------------------------------------------

    def _scan_rust_files(self, root_dir: Path, exclude_tests: bool = True) -> List[str]:
        rust_files = []
        for root, dirs, files in os.walk(root_dir):
            if exclude_tests:
                dirs[:] = [d for d in dirs if 'test' not in d.lower() and 'target' not in d]
            else:
                dirs[:] = [d for d in dirs if 'target' not in d]
            for file in files:
                if file.endswith('.rs'):
                    rust_files.append(os.path.join(root, file))
        return sorted(rust_files)


# ---------------------------------------------------------------------------
# 单文件校验模式
# ---------------------------------------------------------------------------

    def verify_single_file(self, rust_file_path: str,
                           ignored_functions: Set[str] = None) -> Dict:
        if ignored_functions is None:
            ignored_functions = set()

        rust_file_abs = Path(rust_file_path).resolve()

        # 提取相对路径
        parts = rust_file_abs.parts
        if 'src' in parts:
            src_idx = parts.index('src')
            expected_rust_file = str(Path(*parts[src_idx + 1:])).replace('\\', '/')
        else:
            expected_rust_file = rust_file_abs.name

        print(f"=== Single File Verification ===")
        print(f"Input file: {rust_file_path}")
        print(f"Expected rust file pattern: {expected_rust_file}")

        # 查找匹配的黄金清单 entry
        matching_entries = []
        matching_manifest = None

        for manifest_file in MANIFEST_DIR.glob('*.golden.yaml'):
            if '-test.golden.yaml' in manifest_file.name:
                continue
            with open(manifest_file, 'r', encoding='utf-8') as f:
                manifest = yaml.safe_load(f)
            for entry in manifest.get('entries', []):
                if entry.get('rust_file') == expected_rust_file:
                    matching_entries.append(entry)
                    matching_manifest = manifest_file

        if not matching_entries:
            print(f"No matching entry found in golden manifests for: {expected_rust_file}")
            return {
                'metadata': {
                    'verification_mode': 'single_file',
                    'verified_at': datetime.now().isoformat(),
                    'rust_file_abs': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file,
                },
                'summary': {
                    'overall_pass': False,
                    'reason': 'No matching entry found in golden manifests',
                    'issues_found': 1,
                },
                'issues': [{
                    'rust_file': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file,
                    'issue_type': 'NO_MATCHING_ENTRY',
                    'reason': '黄金清单中无对应 entry',
                    'severity': 'HIGH',
                }]
            }

        print(f"Found {len(matching_entries)} matching entries in: {matching_manifest.name}")

        # 提取 Rust 文件符号
        extractor = RustSymbolExtractor()
        rust_symbols = extractor.extract_from_file(str(rust_file_abs))

        if not rust_symbols:
            return {
                'metadata': {
                    'verification_mode': 'single_file',
                    'verified_at': datetime.now().isoformat(),
                    'rust_file_abs': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file,
                },
                'summary': {
                    'overall_pass': False,
                    'reason': 'Failed to extract Rust symbols',
                    'issues_found': 1,
                },
                'issues': [{
                    'rust_file': str(rust_file_abs),
                    'issue_type': 'SYMBOL_EXTRACTION_FAILED',
                    'reason': '无法从 Rust 文件提取符号',
                    'severity': 'HIGH',
                }]
            }

        # 收集该文件的所有函数名
        file_fn_names = set()
        for f in rust_symbols.get('functions', []):
            file_fn_names.add(f['name'])
        for impl_fns in rust_symbols.get('impl_functions', {}).values():
            file_fn_names.update(impl_fns)

        issues = []
        verified_functions = []

        for entry in matching_entries:
            c_file = entry.get('c_file', '')
            for func in entry.get('functions', []):
                c_fn = func['name']
                if _is_function_ignored(c_fn, ignored_functions):
                    continue
                candidates = c_fn_candidates(c_fn)
                matched = any(c in file_fn_names for c in candidates)
                if not matched:
                    issues.append({
                        'c_file': c_file,
                        'c_function': c_fn,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': str(rust_file_abs),
                        'issue_type': 'FUNCTION_MISSING',
                        'reason': f'Rust 缺失函数: {c_fn}',
                        'severity': 'HIGH' if func.get('public', False) else 'MEDIUM',
                        'is_public': func.get('public', False),
                    })
                else:
                    verified_functions.append(c_fn)

        issues_count = len(issues)
        overall_pass = issues_count == 0

        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        output_filename = rust_file_abs.stem + '-single-parity.json'
        output_path = EVIDENCE_DIR / output_filename

        report = {
            'metadata': {
                'verification_mode': 'single_file',
                'verified_at': datetime.now().isoformat(),
                'rust_file_abs': str(rust_file_abs),
                'expected_rust_file': expected_rust_file,
                'matching_manifest': matching_manifest.name if matching_manifest else None,
                'tree_sitter_used': True,
            },
            'summary': {
                'matching_entries': len(matching_entries),
                'verified_functions': len(verified_functions),
                'issues_found': issues_count,
                'overall_pass': overall_pass,
            },
            'verified_functions': verified_functions,
            'issues': issues,
            'issue_breakdown': {
                'FUNCTION_MISSING': len([i for i in issues if i['issue_type'] == 'FUNCTION_MISSING']),
            }
        }

        with open(output_path, 'w', encoding='utf-8') as f:
            json.dump(report, f, indent=2, ensure_ascii=False)

        print(f"\n=== Single File Verification Report ===")
        print(f"Output: {output_path}")
        print(f"Verified functions: {len(verified_functions)}")
        print(f"Issues found: {issues_count}")
        print(f"Overall pass: {overall_pass}")

        if issues:
            print(f"\nIssues:")
            for issue in issues[:10]:
                print(f"  - [{issue['severity']}] {issue['issue_type']}: {issue.get('c_function', 'N/A')} - {issue['reason']}")

        return report


def main():
    parser = argparse.ArgumentParser(
        description='Verify Dual Manifest Parity for FlashDB C→Rust (source + test)'
    )

    mode_group = parser.add_mutually_exclusive_group(required=True)
    mode_group.add_argument('--module', help='Module name (e.g., flashdb) for full module verification')
    mode_group.add_argument('--single-file', help='Single Rust file path for targeted verification')

    parser.add_argument('--rust-root', help='Rust root directory (e.g., .). Required when using --module')
    parser.add_argument('--ignores', help='Path to ignore rules YAML file')

    args = parser.parse_args()

    verifier = ManifestParityVerifier()

    ignored_functions = set()
    ignored_tests = set()
    if args.ignores:
        ignored_functions, ignored_tests = verifier.load_ignores(args.ignores)

    if args.single_file:
        report = verifier.verify_single_file(args.single_file, ignored_functions)
        overall_pass = report['summary'].get('overall_pass', False)
        sys.exit(0 if overall_pass else 1)

    if not args.rust_root:
        parser.error('--rust-root is required when using --module')

    source_report, test_report = verifier.verify_dual(
        args.module, args.rust_root, ignored_functions, ignored_tests
    )

    overall_pass = (
        source_report['summary'].get('overall_pass', False)
        and test_report['summary'].get('overall_pass', False)
    )
    sys.exit(0 if overall_pass else 1)


if __name__ == '__main__':
    main()
