#!/usr/bin/env python3
"""
Golden Manifest Builder (双模式自动生成)
从Java源码和测试代码生成黄金清单（强制tree-sitter）

安装： pip install tree-sitter tree-sitter-java tree-sitter-rust pyyaml

重要约束：
- 强制依赖tree-sitter，无fallback模式
- 自动生成源码清单和测试清单
- 输出路径固定：{module}.golden.yaml 和 {module}-test.golden.yaml

用法：
    python build_golden_manifest.py --module runtime --java-root connect/runtime/src

输出：
    - manifests/runtime.golden.yaml     （源码清单）
    - manifests/runtime-test.golden.yaml（测试清单）
"""

import os
import sys
import re
import yaml
import argparse
from pathlib import Path
from datetime import datetime
from typing import Dict, List, Tuple

# 强制依赖tree-sitter
try:
    import tree_sitter_java as tsjava
    from tree_sitter import Language, Parser
except ImportError:
    print("Error: tree-sitter-java not installed!")
    print("Install: pip install tree-sitter tree-sitter-java")
    sys.exit(1)

# 项目根目录
PROJECT_ROOT = Path(__file__).parent.parent.parent.parent.resolve()
MANIFEST_DIR = PROJECT_ROOT / ".opencode" / "harness" / "manifests"


class JavaSymbolExtractor:
    """从Java源码提取符号（强制tree-sitter）"""
    
    # Java标准Object方法和程序入口方法，在提取时应剔除
    JAVA_STANDARD_METHODS = {'main', 'toString', 'equals', 'hashCode', 
                              'clone', 'getClass', 'close'}
    
    def __init__(self):
        self.parser = Parser(Language(tsjava.language()))
    
    def extract_from_file(self, file_path):
        """从单个Java文件提取符号（使用原始字节避免BOM偏移问题）"""
        try:
            with open(file_path, 'rb') as f:
                content_bytes = f.read()
            
            return self._extract_with_tree_sitter_bytes(content_bytes)
        except Exception as e:
            print(f"Error extracting from {file_path}: {e}")
            return None
    
    def _extract_with_tree_sitter_bytes(self, content_bytes):
        """使用tree-sitter从原始字节进行AST级提取"""
        tree = self.parser.parse(content_bytes)
        root = tree.root_node
        
        symbols = {
            'classes': [],  # 每个类包含: {'name': xxx, 'extends': xxx, 'implements': [...], 'methods': [...]}
            'interfaces': [],  # 每个接口包含: {'name': xxx, 'methods': [...]}
            'enums': []
        }
        
        self._traverse_ast_bytes(root, symbols, content_bytes, current_class=None, current_interface=None)
        return symbols
    
    def _traverse_ast_bytes(self, node, symbols, content_bytes, current_class=None, current_interface=None):
        """递归遍历AST节点（使用原始字节）
        
        Args:
            node: AST节点
            symbols: 符号收集字典
            content_bytes: 文件原始字节
            current_class: 当前遍历所在的类名（用于记录方法所属类）
            current_interface: 当前遍历所在的接口名（用于记录方法所属接口）
        """
        node_type = node.type
        
        if node_type == 'class_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                class_name = name_bytes.decode('utf-8', errors='replace')
                
                # 提取extends（父类）- tree-sitter-java中是superclass节点
                extends_class = None
                for child in node.children:
                    if child.type == 'superclass':
                        # superclass节点内部有type_identifier
                        for ext_child in child.children:
                            if ext_child.type == 'type_identifier':
                                ext_bytes = content_bytes[ext_child.start_byte:ext_child.end_byte]
                                extends_class = ext_bytes.decode('utf-8', errors='replace')
                                break
                
                # 提取implements（接口列表）- tree-sitter-java中是super_interfaces节点
                implements_interfaces = []
                for child in node.children:
                    if child.type == 'super_interfaces':
                        # super_interfaces节点内部有type_list，包含多个type_identifier
                        for imp_child in child.children:
                            if imp_child.type == 'type_list':
                                for type_child in imp_child.children:
                                    if type_child.type == 'type_identifier':
                                        imp_bytes = content_bytes[type_child.start_byte:type_child.end_byte]
                                        implements_interfaces.append(imp_bytes.decode('utf-8', errors='replace'))
                
                # 添加类信息，包含独立的方法列表
                symbols['classes'].append({
                    'name': class_name,
                    'extends': extends_class,
                    'implements': implements_interfaces,
                    'methods': []  # 每个类的独立方法列表
                })
                
                # 递归遍历类内部，传递新的 current_class
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_class=class_name, current_interface=None)
                return  # 类内部已递归处理，直接返回
        
        elif node_type == 'interface_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                interface_name = name_bytes.decode('utf-8', errors='replace')
                
                # 添加接口信息，包含独立的方法列表
                symbols['interfaces'].append({
                    'name': interface_name,
                    'methods': []  # 每个接口的独立方法列表
                })
                
# 递归遍历接口内部，传递新的 current_interface
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_class=None, current_interface=interface_name)
                return  # 接口内部已递归处理，直接返回
        
        elif node_type == 'record_declaration':
            # Java 14+ record语法：public record RecordName(...) { }
            # 在黄金清单中作为class处理（record本质是特殊类）
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                record_name = name_bytes.decode('utf-8', errors='replace')
                
                # 提取implements（record可以implements接口）
                implements_interfaces = []
                for child in node.children:
                    if child.type == 'super_interfaces':
                        for imp_child in child.children:
                            if imp_child.type == 'type_list':
                                for type_child in imp_child.children:
                                    if type_child.type == 'type_identifier':
                                        imp_bytes = content_bytes[type_child.start_byte:type_child.end_byte]
                                        implements_interfaces.append(imp_bytes.decode('utf-8', errors='replace'))
                
                # record作为类处理，record没有extends（隐式继承java.lang.Record）
                symbols['classes'].append({
                    'name': record_name,
                    'extends': None,  # record隐式继承java.lang.Record
                    'implements': implements_interfaces,
                    'methods': []  # record的compact methods在body中
                })
                
                # 递归遍历record内部，传递新的 current_class
                for child in node.children:
                    self._traverse_ast_bytes(child, symbols, content_bytes, current_class=record_name, current_interface=None)
                return  # record内部已递归处理，直接返回
        
        elif node_type == 'enum_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                symbols['enums'].append(name_bytes.decode('utf-8', errors='replace'))
        
        elif node_type == 'method_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                method_name = name_bytes.decode('utf-8', errors='replace')
                
                # 过滤掉Java标准方法（main, toString, equals, hashCode等）
                if method_name in self.JAVA_STANDARD_METHODS:
                    # 递归遍历子节点，跳过方法添加
                    for child in node.children:
                        self._traverse_ast_bytes(child, symbols, content_bytes, current_class=current_class, current_interface=current_interface)
                    return
                
                # 将方法添加到当前类或接口的方法列表
                if current_class:
                    # 找到对应的类并添加方法
                    for cls in symbols['classes']:
                        if cls['name'] == current_class:
                            cls['methods'].append(method_name)
                            break
                elif current_interface:
                    # 找到对应的接口并添加方法
                    for iface in symbols['interfaces']:
                        if iface['name'] == current_interface:
                            iface['methods'].append(method_name)
                            break
        
        # 递归遍历其他节点的子节点，保持 current_class/current_interface
        for child in node.children:
            self._traverse_ast_bytes(child, symbols, content_bytes, current_class=current_class, current_interface=current_interface)


class JavaTestSymbolExtractor:
    """从Java测试文件提取测试符号（层级1+2，强制tree-sitter）"""
    
    def __init__(self):
        self.parser = Parser(Language(tsjava.language()))
    
    def extract_from_file(self, file_path):
        """从单个Java测试文件提取测试符号（使用原始字节避免BOM偏移问题）"""
        try:
            with open(file_path, 'rb') as f:
                content_bytes = f.read()
            
            return self._extract_test_with_tree_sitter_bytes(content_bytes, file_path)
        except Exception as e:
            print(f"Error extracting from {file_path}: {e}")
            return None
    
    def _extract_test_with_tree_sitter_bytes(self, content_bytes, file_path):
        """使用tree-sitter从原始字节提取测试元数据"""
        tree = self.parser.parse(content_bytes)
        root = tree.root_node
        
        test_metadata = {
            'test_class': None,
            'test_file': None,
            'test_methods': []
        }
        
        self._traverse_test_ast_bytes(root, test_metadata, content_bytes, file_path)
        return test_metadata
    
    def _traverse_test_ast_bytes(self, node, test_metadata, content_bytes, file_path):
        """递归遍历AST节点提取测试信息（使用原始字节）"""
        node_type = node.type
        
        if node_type == 'class_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                class_name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                test_metadata['test_class'] = class_name_bytes.decode('utf-8', errors='replace')
                test_metadata['test_file'] = file_path
        
        elif node_type == 'method_declaration':
            name_node = node.child_by_field_name('name')
            if name_node:
                method_name_bytes = content_bytes[name_node.start_byte:name_node.end_byte]
                method_name = method_name_bytes.decode('utf-8', errors='replace')
                
                # 检查是否为测试方法（仅依赖@Test注解）
                annotations = self._get_method_annotations_bytes(node, content_bytes)
                is_test_method = '@Test' in annotations
                
                if is_test_method:
                    test_metadata['test_methods'].append({
                        'java_method': method_name,
                        'rust_method': self._java_to_rust_method_name(method_name),
                    })
        
        for child in node.children:
            self._traverse_test_ast_bytes(child, test_metadata, content_bytes, file_path)
    
    def _get_method_annotations_bytes(self, node, content_bytes):
        """获取方法的所有注解（从 modifiers 子节点中提取，使用原始字节）"""
        annotations = []
        # 注解在 method_declaration 的 modifiers 子节点内
        for child in node.children:
            if child.type == 'modifiers':
                for mod_child in child.children:
                    if mod_child.type == 'marker_annotation':
                        annotation_bytes = content_bytes[mod_child.start_byte:mod_child.end_byte]
                        annotations.append(annotation_bytes.decode('utf-8', errors='replace'))
                    elif mod_child.type == 'annotation':
                        annotation_bytes = content_bytes[mod_child.start_byte:mod_child.end_byte]
                        annotations.append(annotation_bytes.decode('utf-8', errors='replace'))
        return annotations
    
    def _java_to_rust_method_name(self, java_method):
        """Java方法名转Rust方法名（改进的snake_case转换，支持缩写）
        
        转换规则：
        1. 连续大写字母作为一个整体（如HTTP → http）
        2. 单个大写字母后跟小写字母，插入下划线（如Client → _client）
        3. 混合情况正确处理（如HTTPClient → http_client, parseJSON → parse_json）
        """
        # 先处理连续的大写字母序列（缩写）
        result = java_method
        
        # 匹配连续大写字母后跟小写字母或结尾的情况
        # 例如：HTTPClient中的HTTP, parseJSON中的JSON
        result = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', result)  # ABCDx → ABCD_x
        result = re.sub(r'([a-z\d])([A-Z])', r'\1_\2', result)      # abcD → abc_D
        
        # 将连续大写字母整体转为小写
        result = re.sub(r'([A-Z]+)', lambda m: m.group(1).lower(), result)
        
        # 处理其他大写字母
        result = result.lower()
        
        # 清理多余的下划线
        result = result.strip('_')
        result = result.replace('__', '_')
        
        return result

class GoldenManifestBuilder:
    """构建黄金清单（双模式自动生成）"""
    
    def build_dual(self, module, java_root_input):
        """同时生成源码清单和测试清单
        
        Args:
            module: 模块名（如runtime）
            java_root_input: Java根目录路径，支持两种格式：
                           - 相对路径（如connect/runtime/src），相对于项目根目录
                           - 绝对路径（如C:/kafka1/connect/runtime/src）
        """
        java_root_path = Path(java_root_input)
        if java_root_path.is_absolute():
            java_root_abs = java_root_path.resolve()
        else:
            java_root_abs = (PROJECT_ROOT / java_root_input).resolve()
        
        print(f"=== Building Dual Golden Manifests for {module} ===")
        print(f"Project root: {PROJECT_ROOT}")
        print(f"Java root: {java_root_abs}")
        
        if not java_root_abs.exists():
            print(f"Error: Java root directory not found: {java_root_abs}")
            sys.exit(1)
        
        # 扫描Java文件
        java_source_files = self._scan_java_source_files(java_root_abs)
        java_test_files = self._scan_java_test_files(java_root_abs)
        
        print(f"Found {len(java_source_files)} Java source files")
        print(f"Found {len(java_test_files)} Java test files")
        
        # 动态计算公共父路径（源码和测试分别计算）
        source_common_parent = self._compute_common_parent(java_source_files, java_root_abs)
        test_common_parent = self._compute_common_parent(java_test_files, java_root_abs)
        print(f"Computed source common parent: {source_common_parent}")
        print(f"Computed test common parent: {test_common_parent}")
        
        # 提取源码符号
        source_extractor = JavaSymbolExtractor()
        source_entries = []
        
        for java_file in java_source_files:
            rel_path = os.path.relpath(java_file, java_root_abs)
            symbols = source_extractor.extract_from_file(java_file)
            if symbols:
                entry = self._build_source_entry(module, java_file, rel_path, symbols, source_common_parent)
                source_entries.append(entry)
        
        # 提取测试符号
        test_extractor = JavaTestSymbolExtractor()
        test_entries = []
        
        for java_file in java_test_files:
            rel_path = os.path.relpath(java_file, java_root_abs)
            test_metadata = test_extractor.extract_from_file(java_file)
            if test_metadata and test_metadata['test_methods']:
                entry = self._build_test_entry(module, java_file, rel_path, test_metadata, test_common_parent)
                test_entries.append(entry)
        
        # 构建源码清单
        source_manifest = {
            'metadata': {
                'module': module,
                'manifest_type': 'source',
                'generated_at': datetime.now().isoformat(),
                'java_root': java_root_input,
                'rust_root': f"connect-rust/connect-{module}/src",
                'java_file_count': len(java_source_files),
                'generator': 'build_golden_manifest.py',
                'tree_sitter_used': True
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
                'java_root': java_root_input,
                'rust_root': f"connect-rust/connect-{module}/tests",
                'java_test_file_count': len(java_test_files),
                'generator': 'build_golden_manifest.py',
                'tree_sitter_used': True
            },
            'test_entries': test_entries,
            'summary': self._build_test_summary(test_entries)
        }
        
        # 固定输出路径（module参与拼接）
        MANIFEST_DIR.mkdir(parents=True, exist_ok=True)
        
        source_output = MANIFEST_DIR / f"{module}.golden.yaml"
        test_output = MANIFEST_DIR / f"{module}-test.golden.yaml"
        
        # 写入源码清单
        with open(source_output, 'w', encoding='utf-8') as f:
            yaml.dump(source_manifest, f, default_flow_style=False, allow_unicode=True, sort_keys=False)
        
        # 写入测试清单
        with open(test_output, 'w', encoding='utf-8') as f:
            yaml.dump(test_manifest, f, default_flow_style=False, allow_unicode=True, sort_keys=False)
        
        print(f"\n=== Dual Manifests Generated ===")
        print(f"Source manifest: {source_output}")
        print(f"  Total files: {source_manifest['summary']['file_count']}")
        print(f"  Total classes: {source_manifest['summary']['class_count']}")
        print(f"  Total methods: {source_manifest['summary']['method_count']}")
        
        print(f"\nTest manifest: {test_output}")
        print(f"  Total test files: {test_manifest['summary']['test_file_count']}")
        print(f"  Total test methods: {test_manifest['summary']['test_method_count']}")
        
        return source_manifest, test_manifest
    
    def _scan_java_source_files(self, root_dir):
        """扫描Java源码文件（排除test目录和package-info.java）"""
        java_files = []
        for root, dirs, files in os.walk(root_dir):
            dirs[:] = [d for d in dirs if 'test' not in d.lower()]
            for file in files:
                # 过滤掉package-info.java和测试文件
                if file.endswith('.java') and 'Test' not in file and file != 'package-info.java':
                    java_files.append(os.path.join(root, file))
        return sorted(java_files)
    
    def _scan_java_test_files(self, root_dir):
        """扫描Java测试文件（仅扫描test/java目录下的.java文件，排除package-info.java）"""
        java_test_files = []
        for root, dirs, files in os.walk(root_dir):
            # 统一使用正斜杠进行路径匹配，兼容Windows和Unix
            normalized_root = root.replace('\\', '/').lower()
            
            for file in files:
                # 过滤掉package-info.java，仅匹配 test/java 目录下的文件
                if file.endswith('.java') and file != 'package-info.java':
                    if '/test/java/' in normalized_root:
                        java_test_files.append(os.path.join(root, file))
        return sorted(java_test_files)
    
    def _compute_common_parent(self, java_files, java_root_abs):
        """动态计算所有Java文件的公共父路径
        
        Args:
            java_files: Java文件列表
            java_root_abs: Java根目录绝对路径
            
        Returns:
            公共父路径（如 'org/apache/kafka/connect/' 或 'main/java/org/apache/kafka/connect/mirror/'）
        """
        if not java_files:
            return ''
        
        # 获取所有相对路径
        rel_paths = []
        for java_file in java_files:
            rel_path = os.path.relpath(java_file, java_root_abs).replace('\\', '/')
            rel_paths.append(rel_path)
        
        # 计算最长公共前缀（目录级别）
        first_parts = rel_paths[0].split('/')
        common_parts = []
        
        for i in range(len(first_parts)):
            # 检查所有文件在第i层是否有相同的目录名
            candidate_part = first_parts[i]
            all_match = all(
                p.split('/')[i] == candidate_part 
                for p in rel_paths 
                if i < len(p.split('/'))
            )
            if all_match:
                common_parts.append(candidate_part)
            else:
                break
        
# 公共父路径需保留到.java/.rs文件之前的目录层级
        # 找到最后一个非文件名的公共部分
        # 支持Java和Rust两种文件扩展名
        if common_parts and (common_parts[-1].endswith('.java') or common_parts[-1].endswith('.rs')):
            common_parts = common_parts[:-1]
        
        common_parent = '/'.join(common_parts)
        return common_parent
    
    def _build_source_entry(self, module, java_file, rel_path, symbols, common_parent):
        """构建源码清单条目
        
        Args:
            module: 模块名
            java_file: Java文件绝对路径
            rel_path: 相对于java_root的路径
            symbols: 提取的符号信息（新格式：每个类/接口有独立的methods）
            common_parent: 动态计算的公共父路径
        """
        java_class_name = Path(java_file).stem
        java_file_full = rel_path.replace('\\', '/')
        
        # 计算Rust文件路径：从Java相对路径中删除公共父路径
        compact_rel_path = self._remove_common_parent_path(rel_path, common_parent)
        rust_file_name = self._java_to_rust_filename(java_class_name)
        
        # 构建expected_rust_file
        # compact_rel_path可能还包含.java文件名，需要提取目录部分
        if compact_rel_path:
            # 如果compact_rel_path是 'mirror/CheckpointStore.java'，提取'mirror/'
            # 如果compact_rel_path是 'CheckpointStore.java'，目录为空
            if '/' in compact_rel_path:
                dir_path = compact_rel_path.rsplit('/', 1)[0]
                expected_rust_file = f"{dir_path}/{rust_file_name}"
            else:
                # 只剩文件名，说明没有子目录
                expected_rust_file = rust_file_name
        else:
            expected_rust_file = rust_file_name
        
        # 构建类详情列表（新格式：每个类包含 name, extends, implements, methods）
        class_details = []
        class_parents = {}  # class_name -> parents数组（父类+接口）
        
        for cls in symbols['classes']:
            class_details.append({
                'name': cls['name'],
                'methods': cls['methods']  # 每个类的独立方法列表
            })
            # 构建父类映射
            parents = []
            if cls.get('extends'):
                parents.append(cls['extends'])
            parents.extend(cls.get('implements', []))
            class_parents[cls['name']] = parents
        
        # 构建接口详情列表（新格式：每个接口包含 name, methods）
        interface_details = []
        for iface in symbols['interfaces']:
            if isinstance(iface, dict):
                interface_details.append({
                    'name': iface['name'],
                    'methods': iface['methods']  # 每个接口的独立方法列表
                })
            else:
                # 兼容旧格式字符串（如果有）
                interface_details.append({
                    'name': iface,
                    'methods': []
                })
        
        return {
            'java_file': java_file_full,
            'expected_rust_file': expected_rust_file,
            'symbols': {
                'classes': class_details,       # 新格式：[{name, methods}]
                'parents': class_parents,       # 类名到父类/接口的映射
                'interfaces': interface_details, # 新格式：[{name, methods}]
                'enums': symbols['enums']
            }
            # 删除 method_count 字段
        }
    
    def _build_test_entry(self, module, java_file, rel_path, test_metadata, common_parent):
        """构建测试清单条目（层级1+2，去除公共路径）
        
        Args:
            module: 模块名
            java_file: Java文件绝对路径
            rel_path: 相对于java_root的路径
            test_metadata: 提取的测试元数据
            common_parent: 动态计算的公共父路径
        """
        java_test_class = test_metadata['test_class'] or Path(java_file).stem
        java_test_file_full = rel_path.replace('\\', '/')
        
        # 计算去除公共父路径后的相对路径
        compact_rel_path = self._remove_common_parent_path(rel_path, common_parent)
        rust_test_file_name = self._java_to_rust_filename(java_test_class)
        
        # 构建expected_rust_test_file
        # compact_rel_path可能还包含.java文件名，需要提取目录部分
        if compact_rel_path:
            # 如果compact_rel_path是 'mirror/WorkerSinkTaskTest.java'，提取'mirror/'
            # 如果compact_rel_path是 'WorkerSinkTaskTest.java'，目录为空
            if '/' in compact_rel_path:
                dir_path = compact_rel_path.rsplit('/', 1)[0]
                expected_rust_test_file = f"tests/{dir_path}/{rust_test_file_name}"
            else:
                # 只剩文件名，说明没有子目录
                expected_rust_test_file = f"tests/{rust_test_file_name}"
        else:
            expected_rust_test_file = f"tests/{rust_test_file_name}"
        
        return {
            'test_identity': {
                'java_test_class': java_test_class,
                'java_test_file': java_test_file_full,
                'expected_rust_test_file': expected_rust_test_file
            },
            'test_methods': test_metadata['test_methods'],
            'test_method_count': len(test_metadata['test_methods'])
        }
    
    def _remove_common_parent_path(self, rel_path, common_parent):
        """删除动态计算的公共父路径
        
        Args:
            rel_path: Java文件相对路径
            common_parent: 动态计算的公共父路径
            
        Returns:
            删除公共父路径后的相对路径（可能包含.java文件名）
"""
        normalized = rel_path.replace('\\', '/')
        
        if common_parent and normalized.startswith(common_parent):
            # 删除公共父路径 + '/'
            remaining = normalized[len(common_parent):]
            # 如果以 '/' 开头，删除开头的 '/'
            if remaining.startswith('/'):
                remaining = remaining[1:]
            return remaining
        
        return normalized
    
    def _java_to_rust_filename(self, java_class_name):
        """Java类名转Rust文件名"""
        result = re.sub(r'([A-Z])', r'_\1', java_class_name).lower().strip('_')
        result = result.replace('__', '_')
        return f"{result}.rs"
    
    def _build_source_summary(self, entries):
        """构建源码摘要"""
        # 统计方法总数（从每个类的方法列表计算）
        total_methods = 0
        for e in entries:
            for cls in e['symbols']['classes']:
                total_methods += len(cls.get('methods', []))
            for iface in e['symbols']['interfaces']:
                total_methods += len(iface.get('methods', []))
        
        return {
            'file_count': len(entries),
            'class_count': sum(len(e['symbols']['classes']) for e in entries),
            'interface_count': sum(len(e['symbols']['interfaces']) for e in entries),
            'enum_count': sum(len(e['symbols']['enums']) for e in entries),
            'method_count': total_methods
        }
    
    def _build_test_summary(self, test_entries):
        """构建测试摘要"""
        return {
            'test_file_count': len(test_entries),
            'test_method_count': sum(e['test_method_count'] for e in test_entries)
        }


def main():
    parser = argparse.ArgumentParser(description='Build Dual Golden Manifests (source + test)')
    parser.add_argument('--module', required=True, help='Module name (e.g., runtime)')
    parser.add_argument('--java-root', required=True, help='Java root directory, supports relative path (e.g., connect/runtime/src) or absolute path (e.g., C:/kafka1/connect/runtime/src)')
    
    args = parser.parse_args()
    
    builder = GoldenManifestBuilder()
    builder.build_dual(args.module, args.java_root)


if __name__ == '__main__':
    main()