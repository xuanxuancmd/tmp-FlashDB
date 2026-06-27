#!/usr/bin/env python3
"""
Manifest Parity Verifier (双清单自动校验)
校验Rust实现与黄金清单的一致性（强制tree-sitter）

安装： pip install tree-sitter tree-sitter-rust pyyaml

重要约束：
- 强制依赖tree-sitter，无fallback模式
- 自动校验源码清单和测试清单
- 输出路径固定：{module}-parity.json 和 {module}-test-parity.json

用法：
    python verify_manifest_parity.py --module runtime --rust-root connect-rust/connect-runtime

输出：
    - evidence/runtime-parity.json      （源码校验）
    - evidence/runtime-test-parity.json （测试校验）
"""

import os
import sys
import re
import json
import yaml
import argparse
from pathlib import Path
from datetime import datetime
from typing import Dict, List

# 强制依赖tree-sitter
try:
    import tree_sitter_rust as tsrust
    from tree_sitter import Language, Parser
except ImportError:
    print("Error: tree-sitter-rust not installed!")
    print("Install: pip install tree-sitter tree-sitter-rust")
    sys.exit(1)

# 项目根目录
PROJECT_ROOT = Path(__file__).parent.parent.parent.parent.resolve()
MANIFEST_DIR = PROJECT_ROOT / ".opencode" / "harness" / "manifests"
EVIDENCE_DIR = PROJECT_ROOT / ".opencode" / "harness" / "evidence"


class RustSymbolExtractor:
    """从Rust源码/测试文件提取符号（强制tree-sitter）"""
    
    # Rust构造函数约定名称，在比对时应排除（Java构造函数不单独统计）
    RUST_CONSTRUCTOR_NAMES = {'new', 'default', 'close', 'fmt'}
    
    # Java标准Object方法，在比对时应跳过
    JAVA_STANDARD_METHODS = {'hashCode', 'toString', 'equals', 'clone', 'getClass', 'notify', 'notifyAll', 'wait'}
    
    def __init__(self):
        self.parser = Parser(Language(tsrust.language()))
    
    def extract_from_file(self, file_path):
        """从单个Rust文件提取符号（使用原始字节避免BOM偏移问题）"""
        try:
            with open(file_path, 'rb') as f:
                content_bytes = f.read()
            
            return self._extract_with_tree_sitter_bytes(content_bytes)
        except Exception as e:
            print(f"Error extracting from {file_path}: {e}")
            return None
    
    def _extract_with_tree_sitter_bytes(self, content_bytes):
        """使用tree-sitter从原始字节提取符号"""
        tree = self.parser.parse(content_bytes)
        root = tree.root_node
        
        symbols = {
            'structs': [],  # 改为存储struct详情：{'name': xxx, 'file': xxx}
            'traits': [],
            'enums': [],
            'functions': [],  # 独立函数（不在impl块内）
            'impl_functions': {},  # 按struct分组：{struct_name: [func_names]}
            'test_functions': []
        }
        
        self._traverse_ast_bytes(root, symbols, content_bytes)
        return symbols
    
    def _traverse_ast_bytes(self, node, symbols, content_bytes, current_impl_target=None):
        """递归遍历AST节点（使用原始字节）
        
        Args:
            node: AST节点
            symbols: 符号收集字典
            content_bytes: 文件原始字节
            current_impl_target: 当前impl块所属的struct/trait名称（用于识别impl块内函数）
        """
        node_type = node.type
        
        if node_type == 'struct_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                symbols['structs'].append({'name': name_bytes.decode('utf-8', errors='replace')})
        
        elif node_type == 'trait_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                trait_name = name_bytes.decode('utf-8', errors='replace')
                symbols['traits'].append({'name': trait_name})
                # 递归遍历trait块内部，传递trait_name作为target（类似impl_item）
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_impl_target=trait_name)
                return  # trait_item内部已递归处理，直接返回
        
        elif node_type == 'enum_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                symbols['enums'].append({'name': name_bytes.decode('utf-8', errors='replace')})
        
        elif node_type == 'impl_item':
            # 提取impl块的target（struct名称）
            # impl_item的结构：
            # - impl Struct { ... }          -> 直接impl，有1个type_identifier
            # - impl Trait for Struct { ... } -> trait impl，有2个type_identifier
            impl_target = None
            
            # 收集所有type_identifier子节点（直接子节点，不递归）
            type_identifiers = []
            for child in node.children:
                if child.type == 'type_identifier':
                    ti_bytes = content_bytes[child.start_byte:child.end_byte]
                    type_identifiers.append(ti_bytes.decode('utf-8', errors='replace'))
            
            # 根据type_identifier数量判断impl类型
            if len(type_identifiers) == 1:
                # impl Struct { ... } - 直接impl
                impl_target = type_identifiers[0]
            elif len(type_identifiers) >= 2:
                # impl Trait for Struct { ... } - trait impl
                # 第一个是trait，第二个是struct
                impl_target = type_identifiers[1]
            
            # 递归遍历impl块内部，传递impl_target
            if impl_target:
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_impl_target=impl_target)
            else:
                # 如果没有找到impl_target，仍然继续遍历（不传递impl_target）
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_impl_target=None)
            return  # impl_item内部已递归处理，直接返回
        
        elif node_type == 'function_item':
            name_node = node.child_by_field_name('name')
            if name_node:
                func_name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                func_name = func_name_bytes.decode('utf-8', errors='replace')
                
                # 根据是否在impl块内，分别处理
                if current_impl_target:
                    # impl块内的函数，按struct分组
                    if current_impl_target not in symbols['impl_functions']:
                        symbols['impl_functions'][current_impl_target] = []
                    symbols['impl_functions'][current_impl_target].append(func_name)
                else:
                    # 独立函数（不在impl块内）
                    symbols['functions'].append({'name': func_name})
                
                # 检查是否为测试函数（通过AST节点检查attribute_item）
                if self._is_test_function_bytes(node, content_bytes):
                    func_text_bytes = content_bytes[node.start_byte:node.end_byte]
                    func_text = func_text_bytes.decode('utf-8', errors='replace')
                    assertion_count = self._count_rust_assertions(func_text)
                    fake_assertion_risk = self._detect_fake_assertion(func_text)
                    
                    symbols['test_functions'].append({
                        'rust_function': func_name,
                        'assertion_count': assertion_count,
                        'fake_assertion_risk': fake_assertion_risk
                    })
        
        elif node_type == 'function_signature_item':
            # Trait中的方法签名
            for child in node.children:
                if child.type == 'identifier' or child.type == 'field_identifier':
                    name_bytes = content_bytes[child.start_byte:child.end_byte]
                    func_name = name_bytes.decode('utf-8', errors='replace')
                    
                    if current_impl_target:
                        # impl块内的trait方法签名
                        if current_impl_target not in symbols['impl_functions']:
                            symbols['impl_functions'][current_impl_target] = []
                        symbols['impl_functions'][current_impl_target].append(func_name)
                    else:
                        # 独立的trait方法签名
                        symbols['functions'].append({'name': func_name})
                    break
        
        # 递归遍历子节点（传递current_impl_target）
        for child in node.children:
            self._traverse_ast_bytes(child, symbols, content_bytes, current_impl_target=current_impl_target)
    
    def count_non_constructor_functions(self, functions):
        """计算排除构造函数后的函数数量
        
        Args:
            functions: 函数列表 [{'name': 'xxx'}, ...]
            
Returns:
            排除new/default等构造函数后的函数数量
        """
        return len([f for f in functions if f['name'] not in self.RUST_CONSTRUCTOR_NAMES])
    
    def _is_test_function_bytes(self, node, content_bytes):
        """检查函数是否为测试函数（支持 #[test]、#[tokio::test()] 等，使用原始字节）
        
        修复：检查所有前置attribute_item节点，支持 #[test] #[allow(deprecated)] 多属性场景
        """
        parent = node.parent
        if parent:
            # 收集所有在函数之前的attribute_item节点
            attribute_items = []
            for child in parent.children:
                if child == node:
                    break
                if child.type == 'attribute_item':
                    attribute_items.append(child)
            
            # 检查所有attribute_item节点，寻找包含'test'的
            for attr_item in attribute_items:
                for attr_child in attr_item.children:
                    if attr_child.type == 'attribute':
                        # 递归检查 attribute 节点及其所有子节点，寻找名为 'test' 的 identifier
                        if self._contains_test_identifier(attr_child, content_bytes):
                            return True
        
        return False
    
    def _contains_test_identifier(self, node, content_bytes):
        """递归检查节点及其子节点是否包含名为 'test' 的 identifier
        
        Args:
            node: AST节点
            content_bytes: 文件原始字节
            
        Returns:
            bool: 是否包含名为 'test' 的 identifier
        """
        # 如果当前节点是 identifier，检查其名称
        if node.type == 'identifier':
            id_bytes = content_bytes[node.start_byte:node.end_byte]
            id_name = id_bytes.decode('utf-8', errors='replace')
            if id_name == 'test':
                return True
        
        # 递归检查所有子节点
        for child in node.children:
            if self._contains_test_identifier(child, content_bytes):
                return True
        
        return False
    
    def _count_rust_assertions(self, func_text):
        """统计Rust断言数量
        
        修复：识别helper assertion function calls（如assert_no_error(), assert_error()等）
        """
        assertion_patterns = [
            r'assert(?:_eq|_ne|_matches)!',  # assert_eq!, assert_ne!, assert_matches!
            r'assert!',                       # plain assert! macro (修复: 支持无后缀的assert)
            r'should_panic',
            r'\.is_err\s*\(\)',
            r'\.is_ok\s*\(\)',
            r'\.is_some\s*\(\)',
            r'\.is_none\s*\(\)',
            r'\.unwrap\s*\(\)',
            r'\.expect\s*\(',
            r'panic!',
            r'unreachable!',
            # Helper assertion function calls（命名约定：assert_*, assertion_*）
            r'assert(?:_error|_no_error|_success|_failure|_valid|_invalid|_equals|_true|_false)\s*\(',
            r'assertion(?:_check|_verify|_validate|_expect)\s*\(',
        ]
        
        count = 0
        for pattern in assertion_patterns:
            matches = re.findall(pattern, func_text)
            count += len(matches)
        
        return count
    
    def _detect_fake_assertion(self, func_text):
        """检测虚假断言"""
        # assert!(true) 或 assert!(false)
        if re.search(r'assert!\s*\(\s*(?:true|false)\s*\)', func_text):
            return True
        
        # assert_eq!(a, a)
        if re.search(r'assert_eq!\s*\((\w+),\s*\1\s*\)', func_text):
            return True
        
        # 仅println
        if 'println!' in func_text and self._count_rust_assertions(func_text) == 0:
            return True
        
        # 空方法体
        if len(func_text.strip()) < 50:
            return True
        
        return False


class ManifestParityVerifier:
    """校验Rust实现与黄金清单的一致性（双清单自动校验）"""
    
    def verify_dual(self, module, rust_root_rel):
        """同时校验源码清单和测试清单
        
        Args:
            module: 模块名（如runtime）
            rust_root_rel: Rust根目录相对路径（如connect-rust/connect-runtime）
        """
        rust_root_abs = (PROJECT_ROOT / rust_root_rel).resolve()
        
        print(f"=== Verifying Dual Manifest Parity for {module} ===")
        print(f"Project root: {PROJECT_ROOT}")
        print(f"Rust root: {rust_root_abs}")
        
        # 检查清单文件
        source_manifest_path = MANIFEST_DIR / f"{module}.golden.yaml"
        test_manifest_path = MANIFEST_DIR / f"{module}-test.golden.yaml"
        
        if not source_manifest_path.exists():
            print(f"Warning: Source manifest not found: {source_manifest_path}")
        
        if not test_manifest_path.exists():
            print(f"Warning: Test manifest not found: {test_manifest_path}")
        
        # 校验源码清单
        if source_manifest_path.exists():
            source_report = self._verify_source_manifest(source_manifest_path, rust_root_abs / "src")
        else:
            source_report = {'summary': {'overall_pass': False, 'reason': 'Source manifest not found'}}
        
        # 校验测试清单
        if test_manifest_path.exists():
            test_report = self._verify_test_manifest(test_manifest_path, rust_root_abs / "tests", rust_root_abs / "src")
        else:
            test_report = {'summary': {'overall_pass': False, 'reason': 'Test manifest not found'}}
        
        # 固定输出路径（module参与拼接）
        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        
        source_output = EVIDENCE_DIR / f"{module}-parity.json"
        test_output = EVIDENCE_DIR / f"{module}-test-parity.json"
        
        # 写入源码校验报告
        with open(source_output, 'w', encoding='utf-8') as f:
            json.dump(source_report, f, indent=2, ensure_ascii=False)
        
        # 写入测试校验报告
        with open(test_output, 'w', encoding='utf-8') as f:
            json.dump(test_report, f, indent=2, ensure_ascii=False)
        
        print(f"\n=== Dual Verification Reports Generated ===")
        print(f"Source parity: {source_output}")
        print(f"  Issues found: {source_report['summary'].get('issues_found', 0)}")
        print(f"  Overall pass: {source_report['summary'].get('overall_pass', False)}")
        
        print(f"\nTest parity: {test_output}")
        print(f"  Issues found: {test_report['summary'].get('issues_found', 0)}")
        print(f"  Overall pass: {test_report['summary'].get('overall_pass', False)}")
        
        return source_report, test_report
    
    def _verify_source_manifest(self, manifest_path, rust_src_root):
        """校验源码清单"""
        print(f"\n--- Verifying Source Manifest ---")
        
        with open(manifest_path, 'r', encoding='utf-8') as f:
            manifest = yaml.safe_load(f)
        
        if not rust_src_root.exists():
            return {
                'metadata': {'manifest_type': 'source', 'verified_at': datetime.now().isoformat()},
                'summary': {'overall_pass': False, 'reason': f'Rust src root not found: {rust_src_root}'}
            }
        
        # 扫描Rust源码文件
        rust_files = self._scan_rust_files(rust_src_root, exclude_tests=True)
        print(f"Found {len(rust_files)} Rust source files")
        
        # 提取Rust符号
        extractor = RustSymbolExtractor()
        rust_symbols = {}
        
        for rust_file in rust_files:
            symbols = extractor.extract_from_file(rust_file)
            if symbols:
                rust_symbols[rust_file] = symbols
        
# 收集Rust文件路径
        rust_file_paths = {}
        for rf in rust_files:
            path_obj = Path(rf)
            parts = path_obj.parts
            if 'src' in parts:
                src_idx = parts.index('src')
                rel_parts = parts[src_idx + 1:]
                rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                rust_file_paths[rel_fp] = rf
        
        # 建立struct_name到rust_file的映射（用于父类查找）
        # struct_name -> [(rust_file_rel_path, rust_file_abs)]
        all_rust_structs_map = {}
        for rust_file_abs, symbols_data in rust_symbols.items():
            structs = symbols_data.get('structs', [])
            for struct_info in structs:
                struct_name = struct_info['name']
                path_obj = Path(rust_file_abs)
                parts = path_obj.parts
                if 'src' in parts:
                    src_idx = parts.index('src')
                    rel_parts = parts[src_idx + 1:]
                    rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                    if struct_name not in all_rust_structs_map:
                        all_rust_structs_map[struct_name] = []
                    all_rust_structs_map[struct_name].append(rel_fp)
        
        # 建立class_name到rust_file的映射（优先expected_rust_file，其次struct查找）
        # class_name -> (rust_file_rel_path, rust_file_abs, found_in_parent_file)
        class_to_rust_file = {}  # class_name -> (rust_file_rel, rust_file_abs, found_via_parent)
        
        # 首先遍历所有manifest entries建立parents映射
        all_class_parents = {}  # class_name -> parents数组（父类+接口）
        for entry in manifest['entries']:
            if 'symbols' in entry and 'parents' in entry['symbols']:
                for class_name, parents in entry['symbols']['parents'].items():
                    all_class_parents[class_name] = parents
        
        # 定义辅助函数：查找class对应的rust file
        def find_rust_file_for_class(class_name, expected_rust_file, all_rust_structs_map, rust_file_paths, all_class_parents, visited=None):
            """查找class对应的rust file
            
            Args:
                class_name: Java类名
                expected_rust_file: 清单中的expected_rust_file
                all_rust_structs_map: struct_name到rust_file的映射
                rust_file_paths: rust文件路径映射
                all_class_parents: 类到父类的映射
                visited: 已访问的类（防止循环）
                
            Returns:
                (rust_file_rel, rust_file_abs, found_via_parent) 或 None
            """
            if visited is None:
                visited = set()
            
            if class_name in visited:
                return None
            visited.add(class_name)
            
            # 1. 首先尝试expected_rust_file
            if expected_rust_file in rust_file_paths:
                return (expected_rust_file, rust_file_paths[expected_rust_file], False)
            
            # 2. 尝试在expected_rust_file对应的文件中查找struct
            if expected_rust_file in rust_file_paths:
                rust_file_abs = rust_file_paths[expected_rust_file]
                symbols_data = rust_symbols.get(rust_file_abs, {})
                for struct_info in symbols_data.get('structs', []):
                    if struct_info['name'] == class_name:
                        return (expected_rust_file, rust_file_abs, False)
            
            # 3. 根据类名转换为rust文件名尝试查找
            rust_file_name = self._java_class_to_rust_filename(class_name)
            if rust_file_name in rust_file_paths:
                return (rust_file_name, rust_file_paths[rust_file_name], False)
            
            # 4. 在all_rust_structs_map中查找struct名称
            if class_name in all_rust_structs_map:
                rust_file_rel = all_rust_structs_map[class_name][0]  # 取第一个
                return (rust_file_rel, rust_file_paths[rust_file_rel], False)
            
            # 5. 递归查找父类的rust file（如果父类rs文件中包含该struct）
            parents = all_class_parents.get(class_name, [])
            for parent_class in parents:
                # 父类的expected_rust_file需要通过父类名推导
                parent_rust_file_name = self._java_class_to_rust_filename(parent_class)
                result = find_rust_file_for_class(class_name, parent_rust_file_name, all_rust_structs_map, rust_file_paths, all_class_parents, visited)
                if result:
                    return (result[0], result[1], True)  # found_via_parent=True
            
            return None
        
        # 识别问题
        issues = []

        # 收集黄金清单中的所有 expected_rust_file 和 Java类名
        expected_files = set()
        expected_classes = set()  # 收集所有Java类名和接口名（用于检测冗余，排除ignore）
        ignored_classes = set()   # 收集ignore的Java类名（排除冗余检测）
        ignored_entries = 0
        
        for entry in manifest['entries']:
            java_file = entry['java_file']
            expected_rust_file = entry['expected_rust_file']
            expected_files.add(expected_rust_file)

            # 收集Java类名和父类信息
            java_classes = []
            class_parents = {}
            
            if 'symbols' in entry:
                if 'classes' in entry['symbols']:
                    # 新格式：classes 是 [{name, methods}] 列表
                    for java_class_info in entry['symbols']['classes']:
                        # 提取类名和方法列表
                        if isinstance(java_class_info, dict):
                            java_class_name = java_class_info.get('name')
                            java_class_methods_raw = java_class_info.get('methods', [])
                            java_class_ignore = java_class_info.get('ignore', False)
                        else:
                            # 兼容旧格式（纯字符串）
                            java_class_name = java_class_info
                            java_class_methods_raw = []
                            java_class_ignore = False
                        
                        # 解析methods数组（支持字符串和对象两种格式）
                        java_class_methods = []
                        for m in java_class_methods_raw:
                            if isinstance(m, str):
                                # 字符串格式：直接是方法名
                                java_class_methods.append(m)
                            elif isinstance(m, dict):
                                # 对象格式：{方法名: {ignore: true/false}}
                                # 或 loader: {ignore: true}
                                for method_name, method_info in m.items():
                                    if isinstance(method_info, dict) and method_info.get('ignore', False):
                                        # 被ignore的方法，跳过
                                        continue
                                    else:
                                        java_class_methods.append(method_name)
                        
                        # 检查ignore来源：
                        # 1. entry级别ignore标记
                        # 2. 类级别ignore标记（新格式）
                        if entry.get('ignore', False) or java_class_ignore:
                            ignored_classes.add(java_class_name)
                        else:
                            expected_classes.add(java_class_name)
                        java_classes.append({'name': java_class_name, 'methods': java_class_methods, 'ignore': java_class_ignore})
                
                if 'interfaces' in entry['symbols']:
                    # 新格式：interfaces 是 [{name, methods}] 列表
                    for java_interface_info in entry['symbols']['interfaces']:
                        # 提取接口名和方法列表
                        if isinstance(java_interface_info, dict):
                            java_interface_name = java_interface_info.get('name')
                            java_interface_methods_raw = java_interface_info.get('methods', [])
                            java_interface_ignore = java_interface_info.get('ignore', False)
                        else:
                            # 兼容旧格式（纯字符串）
                            java_interface_name = java_interface_info
                            java_interface_methods_raw = []
                            java_interface_ignore = False
                        
                        # 解析methods数组（支持字符串和对象两种格式）
                        java_interface_methods = []
                        for m in java_interface_methods_raw:
                            if isinstance(m, str):
                                # 字符串格式：直接是方法名
                                java_interface_methods.append(m)
                            elif isinstance(m, dict):
                                # 对象格式：{方法名: {ignore: true/false}}
                                for method_name, method_info in m.items():
                                    if isinstance(method_info, dict) and method_info.get('ignore', False):
                                        # 被ignore的方法，跳过
                                        continue
                                    else:
                                        java_interface_methods.append(method_name)
                        
                        # Java接口在Rust中对应trait，统一收集到expected_classes
                        if entry.get('ignore', False) or java_interface_ignore:
                            ignored_classes.add(java_interface_name)
                        else:
                            expected_classes.add(java_interface_name)
                        
                if 'parents' in entry['symbols']:
                    class_parents = entry['symbols']['parents']
            
# 统计忽略条目数
            if entry.get('ignore', False):
                ignored_entries += 1
                continue
            
            # 对每个Java类进行验证（新格式：java_classes是dict列表）
            for java_class_info in java_classes:
                java_class_name = java_class_info['name']
                java_class_methods = java_class_info['methods']
                java_class_ignore = java_class_info['ignore']
                
                # 跳过被忽略的类（不进行struct/trait验证）
                if java_class_name in ignored_classes or java_class_ignore:
                    continue
                
                # 查找对应的rust file（包括父类查找）
                rust_file_info = find_rust_file_for_class(
                    java_class_name, 
                    expected_rust_file, 
                    all_rust_structs_map, 
                    rust_file_paths, 
                    all_class_parents
                )
                
                # 问题类型1: struct/trait缺失（找不到对应的rust file）
                if not rust_file_info:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'issue_type': 'SYMBOL_MISSING',
                        'reason': f'Rust struct/trait缺失（找不到对应文件）',
                        'severity': 'HIGH',
                        'java_methods': java_class_methods
                    })
                    continue
                
                rust_file_rel, rust_file_abs, found_via_parent = rust_file_info
                
                # 获取struct/trait对应的impl_functions
                symbols_data = rust_symbols.get(rust_file_abs, {})
                impl_functions = symbols_data.get('impl_functions', {})
                
                # 问题类型2: struct/trait不存在（在找到的文件中没有对应struct/trait）
                trait_names = [t['name'] for t in symbols_data.get('traits', [])]
                struct_names = [s['name'] for s in symbols_data.get('structs', [])]
                
                # 检查符号是否存在
                found_as_trait = java_class_name in trait_names
                found_as_struct = java_class_name in struct_names
                symbol_found = found_as_trait or found_as_struct or found_via_parent
                
                if not symbol_found:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': rust_file_rel,
                        'issue_type': 'SYMBOL_NOT_FOUND_IN_FILE',
                        'reason': f'Rust文件中未找到struct或trait: {java_class_name}',
                        'severity': 'HIGH',
                        'java_methods': java_class_methods
                    })
                    continue
                
# 获取Rust impl函数列表
                rust_func_names = set()
                if java_class_name in impl_functions:
                    # 排除构造函数
                    for f in impl_functions[java_class_name]:
                        if f not in extractor.RUST_CONSTRUCTOR_NAMES:
                            rust_func_names.add(f)
                
                # 问题类型3: 逐一比对方法存在性
# 过滤Java标准方法，并去重（处理重载方法）
                seen_methods = set()  # 用于去重
                filtered_java_methods_unique = []
                for m in java_class_methods:
                    if m not in extractor.JAVA_STANDARD_METHODS and m not in seen_methods:
                        seen_methods.add(m)
                        filtered_java_methods_unique.append(m)
                
                # 转换Java方法名到Rust snake_case
                def java_to_rust_method_name(java_method):
                    result = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', java_method)
                    result = re.sub(r'([a-z\d])([A-Z])', r'\1_\2', result)
                    result = re.sub(r'([A-Z]+)', lambda m: m.group(1).lower(), result)
                    result = result.lower().strip('_').replace('__', '_')
                    return result
                
                # 检查Java方法是否在Rust中存在（支持重载方法）
                # 匹配规则：
                # 1. 精准匹配：rust_func_name == java_to_rust_method_name(java_method)
                # 2. 前缀匹配：rust_func_name.startswith(java_to_rust_method_name(java_method) + "_by_")
                missing_methods = []
                missing_methods_rust_names = []  # 下划线风格的函数名（用于展示）
                for java_method in filtered_java_methods_unique:
                    rust_method_name = java_to_rust_method_name(java_method)
                    
                    # 检查是否匹配：精准匹配或前缀匹配（_by_）
                    matched = False
                    if rust_method_name in rust_func_names:
                        matched = True  # 精准匹配
                    else:
                        # 前缀匹配：检查是否有 _by_ 前缀的重载版本
                        prefix = rust_method_name + "_by_"
                        for rust_func in rust_func_names:
                            if rust_func.startswith(prefix):
                                matched = True
                                break
                    
                    if not matched:
                        missing_methods.append(java_method)
                        missing_methods_rust_names.append(rust_method_name)  # 展示下划线风格
                
                if missing_methods:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': rust_file_rel,
                        'found_via_parent': found_via_parent,
                        'issue_type': 'METHOD_MISSING',
                        'reason': f'Rust缺失方法: {", ".join(missing_methods_rust_names[:10])}' + ('...' if len(missing_methods_rust_names) > 10 else ''),
                        'severity': 'MEDIUM',
                        'java_methods_count': len(filtered_java_methods_unique),
                        'rust_func_count': len(rust_func_names),
                        'missing_methods': missing_methods_rust_names  # 下划线风格
                    })
                
                # 问题类型4: 检查Rust是否有Java中没有的方法（冗余方法）
                # 构建所有Java方法对应的Rust方法名集合（包含 _by_ 前缀）
                java_rust_method_names = set()
                for m in filtered_java_methods_unique:
                    base_name = java_to_rust_method_name(m)
                    java_rust_method_names.add(base_name)
                    # 允许 _by_ 前缀的重载版本匹配
                    for rust_func in rust_func_names:
                        if rust_func.startswith(base_name + "_by_"):
                            java_rust_method_names.add(rust_func)
                
                redundant_rust_methods = [f for f in rust_func_names if f not in java_rust_method_names]
                
                # 允许少量冗余方法（Rust可能有辅助方法），阈值设为20%
                if len(redundant_rust_methods) > len(filtered_java_methods_unique) * 0.2 and len(redundant_rust_methods) > 5:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': rust_file_rel,
                        'issue_type': 'REDUNDANT_METHOD',
                        'reason': f'Rust有Java中不存在的方法: {", ".join(redundant_rust_methods[:10])}' + ('...' if len(redundant_rust_methods) > 10 else ''),
                        'severity': 'LOW',
                        'java_methods_count': len(filtered_java_methods_unique),
                        'rust_func_count': len(rust_func_names),
                        'redundant_methods': redundant_rust_methods
                    })
        
        # 问题类型5: 检测冗余文件（Rust中有但黄金清单中没有）
        # 排除特殊文件：lib.rs, mod.rs 等模块声明文件
        # 检查文件名的结尾部分
        excluded_filename_suffixes = {'lib.rs', 'mod.rs', 'main.rs', 'build.rs'}

        redundant_files = []
        for rf in set(rust_file_paths.keys()) - expected_files:
            # 检查是否为特殊文件（结尾匹配）
            filename = rf.split('/')[-1]  # 获取文件名部分
            if filename not in excluded_filename_suffixes:
                # 检查该文件中是否有任何struct在expected_classes中
                rust_file_abs = rust_file_paths[rf]
                rust_symbols_data = rust_symbols.get(rust_file_abs, {})
                structs = rust_symbols_data.get('structs', [])
                has_expected_struct = any(s['name'] in expected_classes for s in structs)
                
                if not has_expected_struct:
                    redundant_files.append(rf)

        for redundant_file in sorted(redundant_files):
            rust_file_abs = rust_file_paths[redundant_file]
            rust_symbols_data = rust_symbols.get(rust_file_abs, {})
            # 统计impl块内的函数总数
            impl_functions = rust_symbols_data.get('impl_functions', {})
            rust_func_count = sum(len([f for f in funcs if f not in extractor.RUST_CONSTRUCTOR_NAMES]) 
                                  for funcs in impl_functions.values())

            issues.append({
                'rust_file': redundant_file,
                'issue_type': 'REDUNDANT_FILE',
                'reason': f'Rust文件冗余（文件中无清单对应的struct）',
                'severity': 'LOW',
                'rust_func_count': rust_func_count
            })

# 问题类型4: 检测冗余struct（Rust文件中的struct在Java中没有对应class/interface）
        # 使用前面已建立的all_rust_structs_map
        # 注意：排除ignored_classes（manifest中ignore=true的类）
        redundant_structs = []
        for struct_name, struct_files in all_rust_structs_map.items():
            if struct_name not in expected_classes and struct_name not in ignored_classes:
                redundant_structs.append({
                    'struct_name': struct_name,
                    'rust_files': struct_files
                })

        for redundant in sorted(redundant_structs, key=lambda x: x['struct_name']):
            issues.append({
                'struct_name': redundant['struct_name'],
                'rust_files': redundant['rust_files'],
                'issue_type': 'REDUNDANT_STRUCT',
                'reason': f'Rust struct冗余（黄金清单中无对应Java类）',
                'severity': 'LOW'
            })
        
        # 构建报告
        total_entries = len(manifest['entries'])
        total_classes = sum(len(e['symbols'].get('classes', [])) for e in manifest['entries'] if not e.get('ignore', False))
        total_interfaces = sum(len(e['symbols'].get('interfaces', [])) for e in manifest['entries'] if not e.get('ignore', False))
        total_symbols = total_classes + total_interfaces
        active_entries = total_entries - ignored_entries  # 有效条目数（排除ignore）
        issues_count = len(issues)
        # 简化度量：直接统计issue数量，完全正确的模块issue应为0
        # 所有severity级别（HIGH/MEDIUM/LOW）都计入issues_found
        
        return {
            'metadata': {
                'manifest_type': 'source',
                'verified_at': datetime.now().isoformat(),
                'rust_root': str(rust_src_root),
                'tree_sitter_used': True
            },
            'summary': {
                'total_java_files': total_entries,
                'total_java_classes': total_classes,
                'total_java_interfaces': total_interfaces,
                'ignored_entries': ignored_entries,
                'active_entries': active_entries,
                'total_rust_files': len(rust_files),
                'total_redundant_files': len(redundant_files),
                'issues_found': issues_count,
                'overall_pass': issues_count == 0  # 无issue即通过
            },
'issues': issues,
            'issue_breakdown': {
                'SYMBOL_MISSING': len([i for i in issues if i['issue_type'] == 'SYMBOL_MISSING']),
                'SYMBOL_NOT_FOUND_IN_FILE': len([i for i in issues if i['issue_type'] == 'SYMBOL_NOT_FOUND_IN_FILE']),
                'METHOD_MISSING': len([i for i in issues if i['issue_type'] == 'METHOD_MISSING']),
                'REDUNDANT_METHOD': len([i for i in issues if i['issue_type'] == 'REDUNDANT_METHOD']),
                'REDUNDANT_FILE': len([i for i in issues if i['issue_type'] == 'REDUNDANT_FILE']),
                'REDUNDANT_STRUCT': len([i for i in issues if i['issue_type'] == 'REDUNDANT_STRUCT'])
            }
        }
    
    def _java_class_to_rust_filename(self, java_class_name):
        """Java类名转Rust文件名
        
        Args:
            java_class_name: Java类名（如 IdentityReplicationPolicy）
            
        Returns:
            Rust文件名（如 identity_replication_policy.rs）
        """
        import re
        result = re.sub(r'([A-Z])', r'_\1', java_class_name).lower().strip('_')
        result = result.replace('__', '_')
        return f"{result}.rs"
    
    def _verify_test_manifest(self, manifest_path, rust_tests_root, rust_src_root):
        """校验测试清单"""
        print(f"\n--- Verifying Test Manifest ---")
        
        with open(manifest_path, 'r', encoding='utf-8') as f:
            manifest = yaml.safe_load(f)
        
        # 扫描Rust测试文件（tests/ + src/ #[cfg(test)]）
        rust_test_files = []
        
        if rust_tests_root.exists():
            for root, dirs, files in os.walk(rust_tests_root):
                dirs[:] = [d for d in dirs if 'target' not in d and 'resources' not in d]
                for file in files:
                    if file.endswith('.rs') and not file.startswith('mod.rs'):
                        rust_test_files.append(os.path.join(root, file))
        
        if rust_src_root.exists():
            for root, dirs, files in os.walk(rust_src_root):
                dirs[:] = [d for d in dirs if 'target' not in d]
                for file in files:
                    if file.endswith('.rs'):
                        file_path = os.path.join(root, file)
                        try:
                            with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
                                if '#[cfg(test)]' in f.read() or '#[test]' in f.read():
                                    rust_test_files.append(file_path)
                        except:
                            pass
        
        print(f"Found {len(rust_test_files)} Rust test files")
        
        # 提取Rust测试符号
        extractor = RustSymbolExtractor()
        rust_test_symbols = {}
        
        for rust_file in rust_test_files:
            symbols = extractor.extract_from_file(rust_file)
            if symbols:
                rust_test_symbols[rust_file] = symbols
        
# 收集Rust测试文件路径
        rust_file_paths = {}
        for rf in rust_test_files:
            path_obj = Path(rf)
            parts = path_obj.parts
            
            if 'tests' in parts:
                tests_idx = parts.index('tests')
                rel_parts = parts[tests_idx + 1:]
                # 保留完整的相对路径（包括子目录）
                rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                rust_file_paths[rel_fp] = rf
            elif 'src' in parts:
                src_idx = parts.index('src')
                rel_parts = parts[src_idx + 1:]
                # 保留完整的相对路径（包括子目录）
                rel_fp = str(Path(*rel_parts)).replace('\\', '/')
                rust_file_paths[rel_fp] = rf
        
        # 识别测试问题
        issues = []
        
# 收集黄金清单中的所有 expected_rust_test_file
        expected_test_files = set()
        ignored_test_entries = 0
        total_expected_test_methods = 0  # 统计所有预期测试方法总数
        
        for entry in manifest['test_entries']:
            # 跳过标记为 ignore 的条目
            if entry.get('ignore', False):
                ignored_test_entries += 1
                continue
            
            test_identity = entry['test_identity']
            java_test_file = test_identity['java_test_file']
            expected_rust_test_file = test_identity['expected_rust_test_file']
            expected_test_files.add(expected_rust_test_file)
            
            java_test_methods = entry['test_methods']
            # 统计预期测试方法总数（过滤ignore的测试方法）
            active_test_methods = [tm for tm in java_test_methods if not tm.get('ignore', False)]
            total_expected_test_methods += len(active_test_methods)
            
            # 问题类型1: 测试文件缺失
            # expected_rust_test_file格式可能是:
            # - "tests/file.rs" (无子目录)
            # - "tests/subdir/file.rs" (有子目录)
            # 需要提取文件路径部分用于匹配
            
            # 提取相对于tests/目录的路径
            if expected_rust_test_file.startswith('tests/'):
                rust_test_file_key = expected_rust_test_file[6:]  # 去掉 "tests/"
            else:
                rust_test_file_key = expected_rust_test_file
            
            if rust_test_file_key not in rust_file_paths:
                issues.append({
                    'java_test_file': java_test_file,
                    'expected_rust_test_file': expected_rust_test_file,
                    'issue_type': 'TEST_FILE_MISSING',
                    'reason': f'Rust测试文件缺失',
                    'severity': 'HIGH',
                    'java_test_method_count': len(java_test_methods)
                })
                continue
            
            # 测试文件存在，检查测试方法
            rust_file_key = rust_file_paths[rust_test_file_key]
            rust_test_functions = rust_test_symbols.get(rust_file_key, {}).get('test_functions', [])
            rust_test_file_rel = rust_test_file_key
            
# 问题类型2: 测试方法缺失
            # 过滤掉标记为 ignore 的测试方法
            active_java_test_methods = [tm for tm in java_test_methods if not tm.get('ignore', False)]
            expected_rust_test_names = set(tm['rust_method'] for tm in active_java_test_methods)
            rust_test_names = set(tf['rust_function'] for tf in rust_test_functions)
            missing_tests = expected_rust_test_names - rust_test_names
            
            if missing_tests:
                issues.append({
                    'java_test_file': java_test_file,
                    'expected_rust_test_file': expected_rust_test_file,
                    'issue_type': 'TEST_METHOD_MISSING',
                    'reason': f'缺失测试方法: {", ".join(missing_tests)}',
                    'severity': 'HIGH'
                })
            
            # 问题类型3: 测试方法数量偏差
            test_count_deviation = abs(len(java_test_methods) - len(rust_test_functions)) / len(java_test_methods) if java_test_methods else 0
            
            if test_count_deviation > 0.3:
                issues.append({
                    'java_test_file': java_test_file,
                    'expected_rust_test_file': expected_rust_test_file,
                    'issue_type': 'TEST_COUNT_MISMATCH',
                    'reason': f'测试方法数量偏差: Java={len(java_test_methods)}, Rust={len(rust_test_functions)}',
                    'severity': 'HIGH',
                    'deviation': f'{test_count_deviation:.2%}'
                })
            
# 逐个测试方法比对断言数量
            for java_tm in java_test_methods:
                # 跳过标记为 ignore 的测试方法
                if java_tm.get('ignore', False):
                    continue
                    
                java_method = java_tm['java_method']
                expected_rust_method = java_tm['rust_method']
                
                # 查找对应的Rust测试函数
                matching_rust_func = None
                for rt_func in rust_test_functions:
                    if rt_func['rust_function'] == expected_rust_method:
                        matching_rust_func = rt_func
                        break
                
                # 问题类型4: 测试方法未实现
                if not matching_rust_func:
                    issues.append({
                        'java_test_file': java_test_file,
                        'expected_rust_test_file': expected_rust_test_file,
                        'java_method': java_method,
                        'expected_rust_method': expected_rust_method,
                        'issue_type': 'TEST_METHOD_NOT_IMPLEMENTED',
                        'reason': f'测试方法未实现',
                        'severity': 'HIGH'
                    })
                    continue
                
                # 问题类型5: Rust测试无断言
                rust_assertion_count = matching_rust_func.get('assertion_count', 0)
                if rust_assertion_count == 0:
                    issues.append({
                        'java_test_file': java_test_file,
                        'expected_rust_test_file': expected_rust_test_file,
                        'java_method': java_method,
                        'expected_rust_method': expected_rust_method,
                        'issue_type': 'RUST_TEST_NO_ASSERTION',
                        'reason': f'Rust测试方法无断言',
                        'severity': 'HIGH'
                    })
                
                # 问题类型6: Rust虚假断言
                if matching_rust_func.get('fake_assertion_risk', False):
                    issues.append({
                        'java_test_file': java_test_file,
                        'expected_rust_test_file': expected_rust_test_file,
                        'java_method': java_method,
                        'expected_rust_method': expected_rust_method,
                        'issue_type': 'RUST_FAKE_ASSERTION',
                        'reason': f'Rust测试存在虚假断言风险',
                        'severity': 'HIGH'
                    })
        
        # 问题类型7: 检测冗余测试文件（Rust中有但黄金清单中没有）
        # 排除特殊文件和BDD测试文件
        excluded_filename_suffixes = {'mod.rs', 'lib.rs', 'cucumber.rs'}
        excluded_dirs = {'bdd', 'cucumber', 'features', 'steps'}
        
        # 收集黄金清单中的所有测试文件路径（去除tests/前缀）
        expected_test_files_normalized = set()
        for ef in expected_test_files:
            # expected_rust_test_file格式可能是:
            # - "tests/file.rs" -> "file.rs"
            # - "tests/subdir/file.rs" -> "subdir/file.rs"
            if ef.startswith('tests/'):
                expected_test_files_normalized.add(ef[6:])  # 去掉 "tests/"
            else:
                expected_test_files_normalized.add(ef)
        
        redundant_test_files = []
        for rf in set(rust_file_paths.keys()) - expected_test_files_normalized:
            # 检查是否为特殊文件（结尾匹配）
            filename = rf.split('/')[-1]  # 获取文件名部分
            # 检查是否在排除目录中
            dir_path = '/'.join(rf.split('/')[:-1]) if '/' in rf else ''
            
            # 排除条件：特殊文件名 或 在BDD相关目录中
            if filename not in excluded_filename_suffixes and dir_path not in excluded_dirs:
                redundant_test_files.append(rf)
        
        for redundant_file in sorted(redundant_test_files):
            rust_file_abs = rust_file_paths[redundant_file]
            rust_test_symbols_data = rust_test_symbols.get(rust_file_abs, {})
            rust_test_func_count = len(rust_test_symbols_data.get('test_functions', []))
            
            issues.append({
                'rust_test_file': redundant_file,
                'issue_type': 'REDUNDANT_TEST_FILE',
                'reason': f'Rust测试文件冗余（黄金清单中无对应Java测试类）',
                'severity': 'LOW',
                'rust_test_func_count': rust_test_func_count
            })
        
        # 构建报告
        total_test_entries = len(manifest['test_entries'])
        active_test_entries = total_test_entries - ignored_test_entries  # 有效条目数（排除ignore）
        issues_count = len(issues)
        # 简化度量：直接统计issue数量，完全正确的模块issue应为0
        # 所有severity级别（HIGH/MEDIUM/LOW）都计入issues_found
        
        return {
            'metadata': {
                'manifest_type': 'test',
                'verified_at': datetime.now().isoformat(),
                'rust_tests_root': str(rust_tests_root),
                'rust_src_root': str(rust_src_root),
                'tree_sitter_used': True
            },
            'summary': {
                'total_java_test_files': total_test_entries,
                'ignored_test_entries': ignored_test_entries,
                'active_test_entries': active_test_entries,
                'total_expected_test_methods': total_expected_test_methods,
                'total_rust_test_files': len(rust_test_files),
                'total_redundant_test_files': len(redundant_test_files),
                'issues_found': issues_count,
                'overall_pass': issues_count == 0  # 无issue即通过
            },
            'issues': issues,
            'issue_breakdown': self._build_test_issue_breakdown(issues)
}
    
    def _scan_rust_files(self, root_dir, exclude_tests=True):
        """扫描Rust文件"""
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
    
    def _build_test_issue_breakdown(self, issues):
        """构建测试问题分类统计"""
        issue_types = [
            'TEST_FILE_MISSING',
            'TEST_METHOD_MISSING',
            'TEST_METHOD_NOT_IMPLEMENTED',
            'TEST_COUNT_MISMATCH',
            'RUST_TEST_NO_ASSERTION',
            'RUST_FAKE_ASSERTION',
            'REDUNDANT_TEST_FILE'
        ]
        
        breakdown = {}
        for issue_type in issue_types:
            count = len([i for i in issues if i['issue_type'] == issue_type])
            if count > 0:
                breakdown[issue_type] = count
        
        return breakdown
    
    def verify_single_file(self, rust_file_path):
        """单文件校验模式
        
        Args:
            rust_file_path: Rust文件的绝对路径或相对路径
            
        功能：
            - 通过expected_rust_file快速匹配黄金清单中的entry
            - 跳过ignore=true的类
            - 跳过methods中标注为ignore的方法
            
        Returns:
            校验报告字典
        """
        # 解析文件路径，提取相对路径
        rust_file_abs = Path(rust_file_path).resolve()
        
        # 尝试从路径中提取expected_rust_file格式
        # 路径可能是：connect-rust/connect-runtime/src/cli/abstract_connect_cli.rs
        # 期望格式：cli/abstract_connect_cli.rs
        
        # 首先尝试找到 'src' 目录的位置
        parts = rust_file_abs.parts
        if 'src' in parts:
            src_idx = parts.index('src')
            expected_rust_file = str(Path(*parts[src_idx + 1:])).replace('\\', '/')
        elif 'connect-rust' in parts:
            # 尝试从connect-rust开始查找
            cr_idx = parts.index('connect-rust')
            # 查找是否有模块目录（如connect-runtime）
            remaining_parts = parts[cr_idx + 1:]
            if remaining_parts and remaining_parts[0].startswith('connect-'):
                module_dir = remaining_parts[0]  # e.g., connect-runtime
                if 'src' in remaining_parts:
                    src_idx_in_remaining = remaining_parts.index('src')
                    expected_rust_file = str(Path(*remaining_parts[src_idx_in_remaining + 1:])).replace('\\', '/')
                else:
                    # 如果没有src，直接使用模块后的路径
                    expected_rust_file = str(Path(*remaining_parts[1:])).replace('\\', '/')
            else:
                expected_rust_file = rust_file_abs.name
        else:
            # 最后fallback：仅使用文件名
            expected_rust_file = rust_file_abs.name
        
        print(f"=== Single File Verification ===")
        print(f"Input file: {rust_file_path}")
        print(f"Expected rust file pattern: {expected_rust_file}")
        
        # 扫描所有黄金清单，查找匹配的entry
        matching_entries = []
        matching_manifest = None
        
        for manifest_file in MANIFEST_DIR.glob('*.golden.yaml'):
            # 排除测试清单（只校验源码清单）
            if '-test.golden.yaml' in manifest_file.name:
                continue
            
            with open(manifest_file, 'r', encoding='utf-8') as f:
                manifest = yaml.safe_load(f)
            
            # 在entries中查找匹配的expected_rust_file
            for entry in manifest.get('entries', []):
                if entry.get('expected_rust_file') == expected_rust_file:
                    matching_entries.append(entry)
                    matching_manifest = manifest_file
        
        if not matching_entries:
            print(f"No matching entry found in golden manifests for: {expected_rust_file}")
            return {
                'metadata': {
                    'verification_mode': 'single_file',
                    'verified_at': datetime.now().isoformat(),
                    'rust_file_abs': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file
                },
                'summary': {
                    'overall_pass': False,
                    'reason': 'No matching entry found in golden manifests',
                    'issues_found': 1
                },
                'issues': [{
                    'rust_file': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file,
                    'issue_type': 'NO_MATCHING_ENTRY',
                    'reason': '黄金清单中无对应entry',
                    'severity': 'HIGH'
                }]
            }
        
        print(f"Found {len(matching_entries)} matching entries in: {matching_manifest.name}")
        
        # 提取Rust文件符号
        extractor = RustSymbolExtractor()
        rust_symbols = extractor.extract_from_file(str(rust_file_abs))
        
        if not rust_symbols:
            print(f"Error: Failed to extract symbols from Rust file")
            return {
                'metadata': {
                    'verification_mode': 'single_file',
                    'verified_at': datetime.now().isoformat(),
                    'rust_file_abs': str(rust_file_abs),
                    'expected_rust_file': expected_rust_file
                },
                'summary': {
                    'overall_pass': False,
                    'reason': 'Failed to extract Rust symbols',
                    'issues_found': 1
                },
                'issues': [{
                    'rust_file': str(rust_file_abs),
                    'issue_type': 'SYMBOL_EXTRACTION_FAILED',
                    'reason': '无法从Rust文件提取符号',
                    'severity': 'HIGH'
                }]
            }
        
        # 对每个匹配的entry进行校验
        issues = []
        verified_classes = []
        skipped_classes = []  # 记录跳过的类（ignore=true）
        skipped_methods = []  # 记录跳过的方法
        
        for entry in matching_entries:
            java_file = entry['java_file']
            entry_ignore = entry.get('ignore', False)
            
            # 如果整个entry被标记为ignore，跳过
            if entry_ignore:
                skipped_classes.append({
                    'java_file': java_file,
                    'reason': 'Entry marked as ignore'
                })
                continue
            
            # 处理classes
            for java_class_info in entry.get('symbols', {}).get('classes', []):
                if isinstance(java_class_info, dict):
                    java_class_name = java_class_info.get('name')
                    java_class_methods_raw = java_class_info.get('methods', [])
                    java_class_ignore = java_class_info.get('ignore', False)
                    java_class_ignore_reason = java_class_info.get('ignore_reason', '')
                else:
                    # 兼容旧格式
                    java_class_name = java_class_info
                    java_class_methods_raw = []
                    java_class_ignore = False
                    java_class_ignore_reason = ''
                
                # 跳过ignore=true的类
                if java_class_ignore:
                    skipped_classes.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'reason': java_class_ignore_reason or 'Class marked as ignore'
                    })
                    continue
                
                # 解析methods，过滤ignore的方法
                java_class_methods = []
                for m in java_class_methods_raw:
                    if isinstance(m, str):
                        java_class_methods.append(m)
                    elif isinstance(m, dict):
                        for method_name, method_info in m.items():
                            if isinstance(method_info, dict) and method_info.get('ignore', False):
                                # 记录跳过的方法
                                skipped_methods.append({
                                    'java_class': java_class_name,
                                    'method': method_name,
                                    'reason': method_info.get('ignore_reason', 'Method marked as ignore')
                                })
                            else:
                                java_class_methods.append(method_name)
                
                # 校验struct/trait存在性
                trait_names = [t['name'] for t in rust_symbols.get('traits', [])]
                struct_names = [s['name'] for s in rust_symbols.get('structs', [])]
                
                found_as_trait = java_class_name in trait_names
                found_as_struct = java_class_name in struct_names
                
                if not found_as_trait and not found_as_struct:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': str(rust_file_abs),
                        'issue_type': 'SYMBOL_NOT_FOUND_IN_FILE',
                        'reason': f'Rust文件中未找到struct或trait: {java_class_name}',
                        'severity': 'HIGH',
                        'java_methods': java_class_methods
                    })
                    continue
                
                verified_classes.append(java_class_name)
                
                # 获取impl函数列表
                impl_functions = rust_symbols.get('impl_functions', {})
                rust_func_names = set()
                if java_class_name in impl_functions:
                    for f in impl_functions[java_class_name]:
                        if f not in extractor.RUST_CONSTRUCTOR_NAMES:
                            rust_func_names.add(f)
                
                # 比对方法
                # 过滤Java标准方法并去重
                seen_methods = set()
                filtered_java_methods = []
                for m in java_class_methods:
                    if m not in extractor.JAVA_STANDARD_METHODS and m not in seen_methods:
                        seen_methods.add(m)
                        filtered_java_methods.append(m)
                
                # Java方法名转Rust snake_case
                def java_to_rust_method_name(java_method):
                    result = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', java_method)
                    result = re.sub(r'([a-z\d])([A-Z])', r'\1_\2', result)
                    result = re.sub(r'([A-Z]+)', lambda m: m.group(1).lower(), result)
                    result = result.lower().strip('_').replace('__', '_')
                    return result
                
                # 检查方法缺失
                missing_methods = []
                for java_method in filtered_java_methods:
                    rust_method_name = java_to_rust_method_name(java_method)
                    matched = False
                    
                    # 精准匹配
                    if rust_method_name in rust_func_names:
                        matched = True
                    else:
                        # 前缀匹配（_by_）
                        prefix = rust_method_name + "_by_"
                        for rust_func in rust_func_names:
                            if rust_func.startswith(prefix):
                                matched = True
                                break
                    
                    if not matched:
                        missing_methods.append(rust_method_name)
                
                if missing_methods:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_class_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': str(rust_file_abs),
                        'issue_type': 'METHOD_MISSING',
                        'reason': f'Rust缺失方法: {", ".join(missing_methods[:10])}' + ('...' if len(missing_methods) > 10 else ''),
                        'severity': 'MEDIUM',
                        'missing_methods': missing_methods
                    })
            
            # 处理interfaces（在Rust中对应trait）
            for java_interface_info in entry.get('symbols', {}).get('interfaces', []):
                if isinstance(java_interface_info, dict):
                    java_interface_name = java_interface_info.get('name')
                    java_interface_methods_raw = java_interface_info.get('methods', [])
                    java_interface_ignore = java_interface_info.get('ignore', False)
                    java_interface_ignore_reason = java_interface_info.get('ignore_reason', '')
                else:
                    java_interface_name = java_interface_info
                    java_interface_methods_raw = []
                    java_interface_ignore = False
                    java_interface_ignore_reason = ''
                
                # 跳过ignore=true的接口
                if java_interface_ignore:
                    skipped_classes.append({
                        'java_file': java_file,
                        'java_class': java_interface_name,
                        'reason': java_interface_ignore_reason or 'Interface marked as ignore'
                    })
                    continue
                
                # 解析methods
                java_interface_methods = []
                for m in java_interface_methods_raw:
                    if isinstance(m, str):
                        java_interface_methods.append(m)
                    elif isinstance(m, dict):
                        for method_name, method_info in m.items():
                            if isinstance(method_info, dict) and method_info.get('ignore', False):
                                skipped_methods.append({
                                    'java_class': java_interface_name,
                                    'method': method_name,
                                    'reason': method_info.get('ignore_reason', 'Method marked as ignore')
                                })
                            else:
                                java_interface_methods.append(method_name)
                
                # 校验trait存在性
                trait_names = [t['name'] for t in rust_symbols.get('traits', [])]
                
                if java_interface_name not in trait_names:
                    issues.append({
                        'java_file': java_file,
                        'java_class': java_interface_name,
                        'expected_rust_file': expected_rust_file,
                        'rust_file': str(rust_file_abs),
                        'issue_type': 'SYMBOL_NOT_FOUND_IN_FILE',
                        'reason': f'Rust文件中未找到trait: {java_interface_name}',
                        'severity': 'HIGH',
                        'java_methods': java_interface_methods
                    })
                    continue
                
                verified_classes.append(java_interface_name)
        
        # 构建报告
        issues_count = len(issues)
        overall_pass = issues_count == 0
        
        # 输出报告
        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        # 使用文件名生成输出文件名
        output_filename = rust_file_abs.stem + '-single-parity.json'
        output_path = EVIDENCE_DIR / output_filename
        
        report = {
            'metadata': {
                'verification_mode': 'single_file',
                'verified_at': datetime.now().isoformat(),
                'rust_file_abs': str(rust_file_abs),
                'expected_rust_file': expected_rust_file,
                'matching_manifest': matching_manifest.name if matching_manifest else None,
                'tree_sitter_used': True
            },
            'summary': {
                'matching_entries': len(matching_entries),
                'verified_classes': len(verified_classes),
                'skipped_classes': len(skipped_classes),
                'skipped_methods': len(skipped_methods),
                'issues_found': issues_count,
                'overall_pass': overall_pass
            },
            'verified_classes': verified_classes,
            'skipped_classes': skipped_classes,
            'skipped_methods': skipped_methods,
            'issues': issues,
            'issue_breakdown': {
                'SYMBOL_NOT_FOUND_IN_FILE': len([i for i in issues if i['issue_type'] == 'SYMBOL_NOT_FOUND_IN_FILE']),
                'METHOD_MISSING': len([i for i in issues if i['issue_type'] == 'METHOD_MISSING'])
            }
        }
        
        with open(output_path, 'w', encoding='utf-8') as f:
            json.dump(report, f, indent=2, ensure_ascii=False)
        
        print(f"\n=== Single File Verification Report ===")
        print(f"Output: {output_path}")
        print(f"Matching entries: {len(matching_entries)}")
        print(f"Verified classes: {len(verified_classes)}")
        print(f"Skipped classes (ignore): {len(skipped_classes)}")
        print(f"Skipped methods (ignore): {len(skipped_methods)}")
        print(f"Issues found: {issues_count}")
        print(f"Overall pass: {overall_pass}")
        
        if skipped_classes:
            print(f"\nSkipped classes:")
            for sc in skipped_classes:
                if 'java_class' in sc:
                    print(f"  - {sc['java_class']}: {sc['reason']}")
                else:
                    print(f"  - Entry: {sc['java_file']}: {sc['reason']}")
        
        if skipped_methods:
            print(f"\nSkipped methods:")
            for sm in skipped_methods[:10]:  # 只显示前10个
                print(f"  - {sm['java_class']}.{sm['method']}: {sm['reason']}")
        
        if issues:
            print(f"\nIssues:")
            for issue in issues[:10]:  # 只显示前10个
                print(f"  - [{issue['severity']}] {issue['issue_type']}: {issue.get('java_class', 'N/A')} - {issue['reason']}")
        
        return report


def main():
    parser = argparse.ArgumentParser(description='Verify Dual Manifest Parity (source + test)')
    
    # 互斥组：--module/--rust-root 模式 vs --single-file 模式
    mode_group = parser.add_mutually_exclusive_group(required=True)
    mode_group.add_argument('--module', help='Module name (e.g., runtime) for full module verification')
    mode_group.add_argument('--single-file', help='Single Rust file path for targeted verification (e.g., connect-rust/connect-runtime/src/cli/abstract_connect_cli.rs)')
    
    # --rust-root 仅在 --module 模式下需要
    parser.add_argument('--rust-root', help='Rust root directory (e.g., connect-rust/connect-runtime). Required when using --module')
    
    args = parser.parse_args()
    
    verifier = ManifestParityVerifier()
    
    # 单文件模式
    if args.single_file:
        report = verifier.verify_single_file(args.single_file)
        overall_pass = report['summary'].get('overall_pass', False)
        sys.exit(0 if overall_pass else 1)
    
    # 全模块模式（原有逻辑）
    if not args.rust_root:
        parser.error('--rust-root is required when using --module')
    
    source_report, test_report = verifier.verify_dual(args.module, args.rust_root)
    
    # 返回退出码（两个都通过才算成功）
    overall_pass = source_report['summary'].get('overall_pass', False) and test_report['summary'].get('overall_pass', False)
    sys.exit(0 if overall_pass else 1)


if __name__ == '__main__':
    main()