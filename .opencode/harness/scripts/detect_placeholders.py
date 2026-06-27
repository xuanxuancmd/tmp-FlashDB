#!/usr/bin/env python3
"""
Placeholder Code Detector (强制tree-sitter，固定输出路径)
检测Rust代码中的占位代码（todo!, unimplemented!, 空函数体等）

安装： pip install tree-sitter tree-sitter-rust

重要约束：
- 强制依赖tree-sitter，无fallback模式
- 输出路径固定：{module}-placeholder.json
- 输出包含函数名，便于定位（行号会随代码修改而变化）

用法：
    python detect_placeholders.py --module runtime --rust-root connect-rust/connect-runtime/src
    python detect_placeholders.py --single-file path/to/file.rs --quiet

输出：
    - evidence/runtime-placeholder.json
"""

import os
import sys
import re
import json
import argparse
from pathlib import Path
from datetime import datetime

# Windows环境强制UTF-8输出
if sys.platform == 'win32':
    import io
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8')
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding='utf-8')

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
EVIDENCE_DIR = PROJECT_ROOT / ".opencode" / "harness" / "evidence"


class PlaceholderDetector:
    """检测占位代码（强制tree-sitter）"""
    
    def __init__(self):
        self.parser = Parser(Language(tsrust.language()))
    
    def detect_in_file(self, file_path):
        """在单个文件中检测占位代码"""
        try:
            with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
                content = f.read()
            
            content_bytes = bytes(content, 'utf8')
            
            # 使用 AST 统一检测所有问题（包含函数名）
            issues = self._detect_all_with_ast(content_bytes)
            
            # 对于 lib.rs 文件，额外检测 deny lints
            if Path(file_path).name == 'lib.rs':
                self._check_deny_lints(content, issues)
            
            return issues
        
        except Exception as e:
            print(f"Error detecting in {file_path}: {e}")
            return None
    
    def _check_deny_lints(self, content, issues):
        """检测 lib.rs 文件是否包含必需的 deny lints"""
        required_lints = [
            '#![deny(clippy::todo)]',
            '#![deny(clippy::unimplemented)]',
            '#![deny(clippy::panic)]',
            '#![deny(clippy::unreachable)]',
            '#![deny(clippy::dbg_macro)]',
        ]
        
        missing_lints = []
        for lint in required_lints:
            if lint not in content:
                missing_lints.append(lint)
        
        if missing_lints:
            issues.append({
                'function': 'lib.rs',
                'issue_type': 'WARNING',
                'pattern': 'MISSING_DENY_LINTS',
                'reason': f'lib.rs 缺少必需的 deny lints: {", ".join(missing_lints)}',
                'severity': 'MEDIUM',
                'content': f'Missing: {len(missing_lints)} lints'
            })
    
    def _detect_all_with_ast(self, content_bytes):
        """使用 AST 统一检测所有占位代码（包含函数名）"""
        tree = self.parser.parse(content_bytes)
        root = tree.root_node
        
        issues = []
        
        # 检测 todo! 和 unimplemented! 宏（阻断性）
        self._detect_macros(root, content_bytes, issues)
        
        # 检测空函数体、可疑返回、函数内的 TODO/FIXME 注释
        self._traverse_for_functions(root, content_bytes, issues)
        
# 检测 BDD 测试中 #[then()] 函数的空断言
        self._detect_bdd_empty_assertions(root, content_bytes, issues)
        
        # 检测 #[test] 函数中仅包含 assert!(true) 的恒真断言
        self._detect_test_tautological_assertions(root, content_bytes, issues)
        
        return issues
    
    def _detect_macros(self, node, content_bytes, issues):
        """检测 todo! 和 unimplemented! 宏"""
        if node.type == 'macro_invocation':
            # 注意：tree-sitter-rust 的字段名是 'macro'，不是 'name'
            macro_name_node = node.child_by_field_name('macro')
            if macro_name_node:
                macro_name = content_bytes[macro_name_node.start_byte:macro_name_node.end_byte].decode('utf8')
                
                # 检查是否为阻断性宏
                blocker_macros = {
                    'todo': 'TODO!() macro - 未实现占位符',
                    'unimplemented': 'unimplemented!() macro - 空实现占位符',
                }
                
                if macro_name in blocker_macros:
                    # 尝试找到宏所在的函数
                    func_name = self._find_enclosing_function(node, content_bytes)
                    
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': f'{macro_name}!',
                        'reason': blocker_macros[macro_name],
                        'severity': 'HIGH',
                        'content': content_bytes[node.start_byte:node.end_byte].decode('utf8')[:100]
                    })
        
        for child in node.children:
            self._detect_macros(child, content_bytes, issues)
    
    def _find_enclosing_function(self, node, content_bytes):
        """从节点向上查找所在的函数名"""
        current = node.parent
        while current:
            if current.type == 'function_item':
                name_node = current.child_by_field_name('name')
                if name_node:
                    return content_bytes[name_node.start_byte:name_node.end_byte].decode('utf8')
            current = current.parent
        return 'Unknown'
    
    def _traverse_for_functions(self, node, content_bytes, issues):
        """遍历AST查找空函数体、可疑返回和函数内注释"""
        if node.type == 'function_item':
            name_node = node.child_by_field_name('name')
            func_name = content_bytes[name_node.start_byte:name_node.end_byte].decode('utf8') if name_node else 'Unknown'
            
            body_node = node.child_by_field_name('body')
            if body_node:
                body_content = content_bytes[body_node.start_byte:body_node.end_byte].decode('utf8')
                
                # 空函数体 {}
                if body_content.strip() == '{}':
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': '{}',
                        'reason': f'空函数体 - 函数"{func_name}"无任何实现',
                        'severity': 'HIGH',
                        'content': '{}'
                    })
                
                # 仅注释函数体
                body_without_comments = re.sub(r'//[^\n]*|/\*.*?\*/', '', body_content, flags=re.DOTALL)
                if body_without_comments.strip() in ['{', '{}', '{ }']:
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': 'comment-only',
                        'reason': f'仅注释函数体 - 函数"{func_name}"只有注释无代码',
                        'severity': 'HIGH',
                        'content': body_content.strip()[:100]
                    })
                
                # 检测仅返回默认值的函数
                self._detect_suspicious_returns(node, content_bytes, issues, func_name)
                
                # 检测函数体内的 TODO/FIXME 注释
                self._detect_function_comments(body_node, content_bytes, issues, func_name)
        
        for child in node.children:
            self._traverse_for_functions(child, content_bytes, issues)
    
    def _detect_function_comments(self, body_node, content_bytes, issues, func_name):
        """检测函数体内的 TODO/FIXME 注释（递归遍历）"""
        def traverse_for_comments(node):
            # 检查行注释
            if node.type == 'line_comment':
                comment_text = content_bytes[node.start_byte:node.end_byte].decode('utf8')
                # 使用正则匹配：支持 //todo, //TODO, // todo, // TODO 等各种格式
                if re.search(r'//\s*TODO', comment_text, re.IGNORECASE):
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': 'TODO_COMMENT',
                        'reason': 'TODO注释 - 待办事项标记',
                        'severity': 'HIGH',
                        'content': comment_text.strip()[:100]
                    })
                elif re.search(r'//\s*FIXME', comment_text, re.IGNORECASE):
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': 'FIXME_COMMENT',
                        'reason': 'FIXME注释 - 待修复标记',
                        'severity': 'HIGH',
                        'content': comment_text.strip()[:100]
                    })
            
            # 检查块注释
            elif node.type == 'block_comment':
                comment_text = content_bytes[node.start_byte:node.end_byte].decode('utf8')
                # 块注释中匹配 TODO/FIXME（不区分大小写）
                if re.search(r'TODO', comment_text, re.IGNORECASE):
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': 'TODO_COMMENT',
                        'reason': 'TODO注释 - 待办事项标记',
                        'severity': 'HIGH',
                        'content': comment_text.strip()[:100]
                    })
                elif re.search(r'FIXME', comment_text, re.IGNORECASE):
                    issues.append({
                        'function': func_name,
                        'issue_type': 'BLOCKER',
                        'pattern': 'FIXME_COMMENT',
                        'reason': 'FIXME注释 - 待修复标记',
                        'severity': 'HIGH',
                        'content': comment_text.strip()[:100]
                    })
            
            # 递归遍历子节点
            for child in node.children:
                traverse_for_comments(child)
        
        traverse_for_comments(body_node)
    
    def _detect_suspicious_returns(self, node, content_bytes, issues, func_name):
        """检测仅返回默认值的函数（AST级别分析）"""
        body_node = node.child_by_field_name('body')
        if not body_node:
            return
        
        # 获取函数体内的实际语句（排除注释、花括号）
        statements = []
        for child in body_node.children:
            # 排除注释节点
            if child.type in ['line_comment', 'block_comment']:
                continue
            # 排除花括号本身
            if child.type == '{' or child.type == '}':
                continue
            # 排除空行（token节点为空）
            if child.type == 'token':
                token_text = content_bytes[child.start_byte:child.end_byte].decode('utf8').strip()
                if not token_text:
                    continue
            statements.append(child)
        
        # 如果函数体只有一条语句，检查是否为返回默认值
        if len(statements) == 1:
            stmt = statements[0]
            
            # 检查是否是 return 语句
            if stmt.type == 'return_expression':
                return_value = self._extract_return_value(stmt, content_bytes)
                
                # 检查是否返回默认值
                if return_value in ['None', 'Default::default()', 'Ok(())', 'Err(())']:
                    issues.append({
                        'function': func_name,
                        'issue_type': 'WARNING',
                        'pattern': f'return {return_value}',
                        'reason': f'函数"{func_name}"仅返回默认值，可能为占位实现',
                        'severity': 'MEDIUM',
                        'content': content_bytes[stmt.start_byte:stmt.end_byte].decode('utf8')[:100]
                    })
            
            # 检查是否是表达式语句（隐式返回，如 `None` 或 `Ok(())`）
            elif stmt.type == 'expression_statement':
                expr = stmt.child_by_field_name('expression')
                if expr:
                    expr_text = content_bytes[expr.start_byte:expr.end_byte].decode('utf8').strip()
                    
                    # 检查是否为 None（隐式返回）
                    if expr_text == 'None':
                        issues.append({
                            'function': func_name,
                            'issue_type': 'WARNING',
                            'pattern': 'None',
                            'reason': f'函数"{func_name}"仅返回None（隐式），可能为占位实现',
                            'severity': 'MEDIUM',
                            'content': expr_text[:100]
                        })
                    
                    # 检查是否为 Ok(())、Err(())、Some(())
                    elif expr.type == 'call_expression':
                        func_name_node = expr.child_by_field_name('function')
                        if func_name_node:
                            func_text = content_bytes[func_name_node.start_byte:func_name_node.end_byte].decode('utf8')
                            if func_text in ['Ok', 'Err', 'Some']:
                                args = expr.child_by_field_name('arguments')
                                if args:
                                    args_text = content_bytes[args.start_byte:args.end_byte].decode('utf8').strip()
                                    if args_text == '()':
                                        issues.append({
                                            'function': func_name,
                                            'issue_type': 'WARNING',
                                            'pattern': f'{func_text}(())',
                                            'reason': f'函数"{func_name}"仅返回{func_text}(())，可能为占位实现',
                                            'severity': 'MEDIUM',
                                            'content': expr_text[:100]
                                        })
    
    def _extract_return_value(self, return_node, content_bytes):
        """提取return语句的返回值"""
        # return_expression 节点结构: "return" + 返回值表达式
        # 需要找到返回值部分
        for child in return_node.children:
            if child.type == 'return':  # 关键字
                continue
            # 返回值表达式
            expr_text = content_bytes[child.start_byte:child.end_byte].decode('utf8').strip()
            return expr_text
        return 'Unknown'
    
    def _detect_bdd_empty_assertions(self, root, content_bytes, issues):
        """检测 BDD 测试中 #[then()] 函数的空断言（没有 assert 宏）"""
        # 断言宏列表
        assertion_macros = ['assert', 'assert_eq', 'assert_ne', 'debug_assert', 
                           'debug_assert_eq', 'debug_assert_ne', 'panic']
        
        def traverse(node):
            if node.type == 'function_item':
                # 检查函数是否有 #[then()] 属性
                has_then_attribute = False
                for child in node.children:
                    if child.type == 'attribute_item':
                        attr_text = content_bytes[child.start_byte:child.end_byte].decode('utf8')
                        # 匹配 #[then(...)] 或 #[then] 格式
                        if re.search(r'#\[then\s*(\(|\])', attr_text):
                            has_then_attribute = True
                            break
                
                if has_then_attribute:
                    # 获取函数名
                    name_node = node.child_by_field_name('name')
                    func_name = content_bytes[name_node.start_byte:name_node.end_byte].decode('utf8') if name_node else 'Unknown'
                    
                    # 获取函数体
                    body_node = node.child_by_field_name('body')
                    if body_node:
                        # 检查函数体内是否有断言宏
                        has_assertion = self._check_body_for_assertions(body_node, content_bytes, assertion_macros)
                        
                        if not has_assertion:
                            issues.append({
                                'function': func_name,
                                'issue_type': 'BLOCKER',
                                'pattern': 'BDD_EMPTY_ASSERTION',
                                'reason': f'BDD测试 #[then()] 函数"{func_name}"没有断言 - 空实现',
                                'severity': 'HIGH',
                                'content': content_bytes[body_node.start_byte:body_node.end_byte].decode('utf8').strip()[:100]
                            })
            
            for child in node.children:
                traverse(child)
        
        traverse(root)
    
    def _check_body_for_assertions(self, body_node, content_bytes, assertion_macros):
        """检查函数体内是否包含断言宏"""
        def check_node(node):
            if node.type == 'macro_invocation':
                # 获取宏名
                macro_name_node = node.child_by_field_name('macro')
                if macro_name_node:
                    macro_name = content_bytes[macro_name_node.start_byte:macro_name_node.end_byte].decode('utf8')
                    if macro_name in assertion_macros:
                        return True
            
            for child in node.children:
                if check_node(child):
                    return True
            
            return False
        
        return check_node(body_node)
    
    def _detect_test_tautological_assertions(self, root, content_bytes, issues):
        """检测 #[test] 函数中仅包含 assert!(true) 的恒真断言（不验证任何行为）
        
        在tree-sitter-rust中，#[test] 是function_item的前驱兄弟节点（attribute_item），不是子节点。
        因此需要遍历root的children，检查attribute_item+function_item的相邻节点对。
        """
        children = root.children
        for i in range(len(children) - 1):
            # 检查 #[test] 属性是否紧跟一个 function_item
            if children[i].type == 'attribute_item':
                attr_text = content_bytes[children[i].start_byte:children[i].end_byte].decode('utf8')
                if re.search(r'#\[test\]', attr_text) and children[i + 1].type == 'function_item':
                    func_node = children[i + 1]
                    name_node = func_node.child_by_field_name('name')
                    func_name = content_bytes[name_node.start_byte:name_node.end_byte].decode('utf8') if name_node else 'Unknown'
                    
                    body_node = func_node.child_by_field_name('body')
                    if body_node:
                        body_text = content_bytes[body_node.start_byte:body_node.end_byte].decode('utf8')
                        
                        # 统计所有 assert! 调用和恒真断言 assert!(true...)
                        all_asserts = list(re.finditer(r'assert!\s*\(', body_text))
                        tautological_asserts = list(re.finditer(r'assert!\s*\(\s*true', body_text))
                        
                        # 如果所有 assert! 调用都是恒真断言（assert!(true) 或 assert!(true, "...")）
                        if len(all_asserts) > 0 and len(tautological_asserts) == len(all_asserts):
                            issues.append({
                                'function': func_name,
                                'issue_type': 'BLOCKER',
                                'pattern': 'TAUTOLOGICAL_ASSERTION',
                                'reason': f'#[test] 函数"{func_name}"仅包含恒真断言 assert!(true) — 不验证任何行为',
                                'severity': 'HIGH',
                                'content': body_text.strip()[:100]
                            })


class PlaceholderChecker:
    """执行占位代码检查（固定输出路径）"""
    
    def check_single_file(self, file_path, quiet=False):
        """检查单个文件（增量检测模式）
        
        Args:
            file_path: 文件路径（支持绝对路径或相对路径）
                       相对路径基于项目根目录（如 connect-rust/connect-api/src/lib.rs）
            quiet: 是否静默模式（只输出问题）
        """
        # 判断路径类型：绝对路径直接使用，相对路径基于项目根目录
        if Path(file_path).is_absolute():
            abs_file_path = Path(file_path)
        else:
            # 相对路径基于项目根目录转换
            abs_file_path = PROJECT_ROOT / file_path
        
        if not abs_file_path.exists():
            if not quiet:
                print(f"Error: File not found: {abs_file_path}")
                print(f"  Input path: {file_path}")
                print(f"  Resolved to: {abs_file_path}")
            sys.exit(1)
        
        detector = PlaceholderDetector()
        issues = detector.detect_in_file(str(abs_file_path))
        
        if not issues or len(issues) == 0:
            if not quiet:
                print(f"[PASS] {abs_file_path}: 无placeholder问题")
            sys.exit(0)
        
        # 有问题 - 输出阻断提示
        blockers = [i for i in issues if i['severity'] == 'HIGH']
        warnings = [i for i in issues if i['severity'] == 'MEDIUM']
        
        if quiet:
            # 静默模式：简洁输出（适合hooks）
            for issue in blockers:
                print(f"[BLOCKER] {abs_file_path}")
                print(f"   函数: {issue['function']}")
                print(f"   问题: {issue['reason']}")
                print(f"   Pattern: {issue['pattern']}")
            
            for issue in warnings:
                print(f"[WARNING] {abs_file_path}")
                print(f"   函数: {issue['function']}")
                print(f"   问题: {issue['reason']}")
        else:
            # 详细模式
            if blockers:
                print(f"[BLOCKER] {abs_file_path} 存在placeholder问题:")
                print(f"   Blockers: {len(blockers)}, Warnings: {len(warnings)}")
                for issue in blockers:
                    print(f"   - [{issue['function']}] {issue['reason']}")
            else:
                print(f"[WARNING] {abs_file_path} 存在潜在问题:")
                print(f"   Warnings: {len(warnings)}")
                for issue in warnings:
                    print(f"   - [{issue['function']}] {issue['reason']}")
        
        sys.exit(1 if len(blockers) > 0 else 0)
    
    def check(self, module, rust_root_rel):
        """执行检查
        
        Args:
            module: 模块名（如runtime）
            rust_root_rel: Rust根目录相对路径（如connect-rust/connect-runtime/src）
        """
        rust_root_abs = (PROJECT_ROOT / rust_root_rel).resolve()
        
        print(f"=== Detecting Placeholders for {module} ===")
        print(f"Project root: {PROJECT_ROOT}")
        print(f"Rust root: {rust_root_abs}")
        
        if not rust_root_abs.exists():
            print(f"Error: Rust root directory not found: {rust_root_abs}")
            sys.exit(1)
        
        # 扫描Rust文件
        rust_files = self._scan_rust_files(rust_root_abs)
        print(f"Found {len(rust_files)} Rust files")
        
        detector = PlaceholderDetector()
        
        # 只收集有问题的文件
        problematic_files = []
        
        for rust_file in rust_files:
            issues = detector.detect_in_file(rust_file)
            
            if issues and len(issues) > 0:
                # 提取相对路径
                rel_path = str(Path(rust_file).relative_to(rust_root_abs)).replace('\\', '/')
                
                # 统计阻断性和警告性问题
                blockers = [i for i in issues if i['severity'] == 'HIGH']
                warnings = [i for i in issues if i['severity'] == 'MEDIUM']
                
                problematic_files.append({
                    'file': rel_path,
                    'blocker_count': len(blockers),
                    'warning_count': len(warnings),
                    'total_issues': len(issues),
                    'issues': issues
                })
        
        # 构建报告
        total_files = len(rust_files)
        files_with_issues = len(problematic_files)
        total_blockers = sum(f['blocker_count'] for f in problematic_files)
        total_warnings = sum(f['warning_count'] for f in problematic_files)
        
        overall_pass = total_blockers == 0
        
        report = {
            'metadata': {
                'module': module,
                'checked_at': datetime.now().isoformat(),
                'rust_root': rust_root_rel,
                'tree_sitter_used': True
            },
            'summary': {
                'total_files_scanned': total_files,
                'files_with_issues': files_with_issues,
                'files_clean': total_files - files_with_issues,
                'total_blockers': total_blockers,
                'total_warnings': total_warnings,
                'overall_pass': overall_pass
            },
            'problematic_files': problematic_files,
            'issue_breakdown': {
                'TODO_MACRO': sum(1 for f in problematic_files for i in f['issues'] if 'todo!' in i['pattern']),
                'UNIMPLEMENTED_MACRO': sum(1 for f in problematic_files for i in f['issues'] if 'unimplemented!' in i['pattern']),
                'EMPTY_BODY': sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == '{}'),
                'COMMENT_ONLY': sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == 'comment-only'),
                'SUSPICIOUS_RETURN': sum(1 for f in problematic_files for i in f['issues'] if 'return' in i['pattern'].lower() or i['pattern'] in ['None', 'Ok(())', 'Err(())', 'Some(())']),
                'TODO_COMMENT': sum(1 for f in problematic_files for i in f['issues'] if 'TODO' in i['pattern'] or 'FIXME' in i['pattern']),
                'BDD_EMPTY_ASSERTION': sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == 'BDD_EMPTY_ASSERTION')
            }
        }
        
        # 固定输出路径（module参与拼接）
        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        output_path = EVIDENCE_DIR / f"{module}-placeholder.json"
        
        # 写入输出
        with open(output_path, 'w', encoding='utf-8') as f:
            json.dump(report, f, indent=2, ensure_ascii=False)
        
        # 打印摘要
        print(f"\n=== Placeholder Check Summary ===")
        print(f"Output: {output_path}")
        print(f"Files with issues: {files_with_issues}/{total_files}")
        print(f"Blockers: {total_blockers}, Warnings: {total_warnings}")
        print(f"Overall pass: {overall_pass}")
        
        if files_with_issues > 0 and files_with_issues <= 20:
            print(f"\nProblematic functions:")
            for f in problematic_files[:20]:
                print(f"\n  File: {f['file']}")
                for issue in f['issues']:
                    severity_str = 'BLOCKER' if issue['severity'] == 'HIGH' else 'WARNING'
                    print(f"    - Function: {issue['function']}")
                    print(f"      Type: {severity_str}")
                    print(f"      Pattern: {issue['pattern']}")
                    print(f"      Reason: {issue['reason']}")
                    if issue['content']:
                        print(f"      Content: {issue['content'][:80]}")
        
        return report
    
    def _scan_rust_files(self, root_dir):
        """扫描Rust文件"""
        rust_files = []
        for root, dirs, files in os.walk(root_dir):
            dirs[:] = [d for d in dirs if 'target' not in d.lower()]
            for file in files:
                if file.endswith('.rs'):
                    rust_files.append(os.path.join(root, file))
        return sorted(rust_files)
    
    def check_bdd(self, module, bdd_root_rel):
        """检测 BDD 测试中的空断言
        
        Args:
            module: 模块名（如 runtime 或 bdd-tests）
            bdd_root_rel: BDD 测试根目录相对路径（如 connect-rust/connect-runtime/tests/bdd）
        """
        bdd_root_abs = (PROJECT_ROOT / bdd_root_rel).resolve()
        
        print(f"=== Detecting BDD Empty Assertions for {module} ===")
        print(f"Project root: {PROJECT_ROOT}")
        print(f"BDD root: {bdd_root_abs}")
        
        if not bdd_root_abs.exists():
            print(f"Error: BDD root directory not found: {bdd_root_abs}")
            sys.exit(1)
        
        # 扫描 BDD 测试文件
        bdd_files = self._scan_rust_files(bdd_root_abs)
        print(f"Found {len(bdd_files)} BDD test files")
        
        detector = PlaceholderDetector()
        
        # 只收集有问题的文件
        problematic_files = []
        
        for bdd_file in bdd_files:
            issues = detector.detect_in_file(bdd_file)
            
            if issues and len(issues) > 0:
                # 提取相对路径
                rel_path = str(Path(bdd_file).relative_to(bdd_root_abs)).replace('\\', '/')
                
                # 统计阻断性和警告性问题
                blockers = [i for i in issues if i['severity'] == 'HIGH']
                warnings = [i for i in issues if i['severity'] == 'MEDIUM']
                
                problematic_files.append({
                    'file': rel_path,
                    'blocker_count': len(blockers),
                    'warning_count': len(warnings),
                    'total_issues': len(issues),
                    'issues': issues
                })
        
        # 构建报告
        total_files = len(bdd_files)
        files_with_issues = len(problematic_files)
        total_blockers = sum(f['blocker_count'] for f in problematic_files)
        total_warnings = sum(f['warning_count'] for f in problematic_files)
        
        # 统计 #[then()] 空断言数量
        bdd_empty_count = sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == 'BDD_EMPTY_ASSERTION')
        
        overall_pass = bdd_empty_count == 0
        
        report = {
            'metadata': {
                'module': module,
                'checked_at': datetime.now().isoformat(),
                'bdd_root': bdd_root_rel,
                'tree_sitter_used': True
            },
            'summary': {
                'total_files_scanned': total_files,
                'files_with_issues': files_with_issues,
                'files_clean': total_files - files_with_issues,
                'total_blockers': total_blockers,
                'total_warnings': total_warnings,
                'bdd_empty_assertions': bdd_empty_count,
                'overall_pass': overall_pass
            },
            'problematic_files': problematic_files,
            'issue_breakdown': {
                'TODO_MACRO': sum(1 for f in problematic_files for i in f['issues'] if 'todo!' in i['pattern']),
                'UNIMPLEMENTED_MACRO': sum(1 for f in problematic_files for i in f['issues'] if 'unimplemented!' in i['pattern']),
                'EMPTY_BODY': sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == '{}'),
                'COMMENT_ONLY': sum(1 for f in problematic_files for i in f['issues'] if i['pattern'] == 'comment-only'),
                'SUSPICIOUS_RETURN': sum(1 for f in problematic_files for i in f['issues'] if 'return' in i['pattern'].lower() or i['pattern'] in ['None', 'Ok(())', 'Err(())', 'Some(())']),
                'TODO_COMMENT': sum(1 for f in problematic_files for i in f['issues'] if 'TODO' in i['pattern'] or 'FIXME' in i['pattern']),
                'BDD_EMPTY_ASSERTION': bdd_empty_count
            }
        }
        
        # 固定输出路径
        EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
        output_path = EVIDENCE_DIR / f"{module}-placeholder.json"
        
        # 写入输出
        with open(output_path, 'w', encoding='utf-8') as f:
            json.dump(report, f, indent=2, ensure_ascii=False)
        
        # 打印摘要
        print(f"\n=== BDD Empty Assertion Check Summary ===")
        print(f"Output: {output_path}")
        print(f"Files with issues: {files_with_issues}/{total_files}")
        print(f"#[then()] empty assertions: {bdd_empty_count}")
        print(f"Overall pass: {overall_pass}")
        
        if bdd_empty_count > 0:
            print(f"\n#[then()] functions without assertions:")
            for f in problematic_files:
                for issue in f['issues']:
                    if issue['pattern'] == 'BDD_EMPTY_ASSERTION':
                        print(f"  - File: {f['file']}")
                        print(f"    Function: {issue['function']}")
                        print(f"    Reason: {issue['reason']}")
        
        return report


def main():
    parser = argparse.ArgumentParser(description='Detect Placeholder Code (fixed output)')
    
    # 单文件模式参数
    parser.add_argument('--single-file', help='Single file path to check (增量检测)')
    parser.add_argument('--quiet', action='store_true', help='Quiet mode - only output issues')
    
    # 模块模式参数（与 --single-file 互斥）
    parser.add_argument('--module', help='Module name (e.g., runtime)')
    parser.add_argument('--rust-root', help='Rust source root (e.g., connect-rust/connect-runtime/src)')
    
    # BDD 测试检测参数
    parser.add_argument('--bdd', action='store_true', help='检测 BDD 测试中的空断言')
    parser.add_argument('--bdd-root', help='BDD tests root directory (e.g., connect-rust/connect-runtime/tests/bdd)')
    
    args = parser.parse_args()
    
    # 参数验证
    if args.single_file:
        # 单文件模式
        checker = PlaceholderChecker()
        checker.check_single_file(args.single_file, args.quiet)
    elif args.bdd and args.bdd_root:
        # BDD 测试检测模式
        checker = PlaceholderChecker()
        report = checker.check_bdd(args.module or 'bdd-tests', args.bdd_root)
        sys.exit(0 if report['summary']['overall_pass'] else 1)
    elif args.module and args.rust_root:
        # 模块模式
        checker = PlaceholderChecker()
        report = checker.check(args.module, args.rust_root)
        sys.exit(0 if report['summary']['overall_pass'] else 1)
    else:
        parser.error("需要提供 --single-file 或 (--module + --rust-root) 或 (--bdd + --bdd-root)")


if __name__ == '__main__':
    main()