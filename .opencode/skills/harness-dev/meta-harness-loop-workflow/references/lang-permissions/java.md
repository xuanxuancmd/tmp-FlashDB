# Java 语言权限片段

> 供 code-executor-agent / code-evaluator-agent 模板的 `{bash_permissions}` 占位符使用。
> 当 `target_lang = java` 时，meta skill 读取本文件，提取下方代码块内容填入模板。
> Java 构建工具（当前仅Maven），根据项目根目录的标识文件选择对应片段。

## 填充规则

- 每个命令必须包含"无参数"和"带参数"两条规则（opencode glob `*` 匹配命令前缀，不匹配空字符串）
- 模板已内置 `"*": "deny"` 兜底（在最前），本文件只提供 allow 白名单
- 本文件已包含通用辅助命令（git/python/ls 等），无需额外拼接

## bash_permissions 片段（Maven，标识文件 `pom.xml`）

```yaml
    "mvn compile": "allow"
    "mvn compile *": "allow"
    "mvn test": "allow"
    "mvn test *": "allow"
    "mvn verify": "allow"
    "mvn verify *": "allow"
    "mvn clean": "allow"
    "mvn clean *": "allow"
    "mvn package": "allow"
    "mvn package *": "allow"
    "git add *": "allow"
    "git commit *": "allow"
    "git status": "allow"
    "git status *": "allow"
    "git diff": "allow"
    "git diff *": "allow"
    "git log": "allow"
    "git log *": "allow"
    "python": "allow"
    "python *": "allow"
    "python3": "allow"
    "python3 *": "allow"
    "ls": "allow"
    "ls *": "allow"
    "dir": "allow"
    "dir *": "allow"
    "cat *": "allow"
    "type *": "allow"
    "Remove-Item*": "allow"
    "del *": "allow"
    "echo *": "allow"
```

## 默认命令

- Maven: `build_cmd` = `mvn compile`，`test_cmd` = `mvn test`
