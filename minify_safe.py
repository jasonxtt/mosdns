import re
import os

# 使用当前目录作为基础目录，这样脚本可以在任何地方运行（只要在项目根目录下）
base_dir = os.getcwd()

def minify_css(content):
    # Remove block comments
    content = re.sub(r'/\*[\s\S]*?\*/', '', content)
    # Normalize whitespace (replace multiple spaces/newlines with single space)
    content = re.sub(r'\s+', ' ', content)
    # Remove space around delimiters
    content = re.sub(r'\s*([:;{}])\s*', r'\1', content)
    # Remove final semicolon in block
    content = re.sub(r';}', '}', content)
    return content.strip()

def minify_js_safe(content):
    """
    Safely minify JS by only removing full-line comments and empty lines.
    Preserves indentation and structure to avoid Automatic Semicolon Insertion (ASI) bugs.
    """
    lines = content.split('\n')
    new_lines = []
    for line in lines:
        stripped = line.strip()
        if not stripped:
            continue
        # Remove lines that look like comments
        if stripped.startswith('//'):
            continue
        # We keep the line as is including whitespace to be safe
        new_lines.append(line.rstrip())
    
    return '\n'.join(new_lines)

# 1. Compress CSS
css_files = [
    r"coremain/www/assets/css/log_refactored.css",
    # Add other CSS files if you want to compress them too
]

for rel_path in css_files:
    abs_path = os.path.join(base_dir, rel_path)
    if os.path.exists(abs_path):
        print(f"Processing CSS: {rel_path}")
        with open(abs_path, 'r', encoding='utf-8') as f:
            raw = f.read()
        minified = minify_css(raw)
        out_path = abs_path.replace('.css', '.min.css')
        with open(out_path, 'w', encoding='utf-8') as f:
            f.write(minified)
        print(f"  -> Minified to {out_path} ({len(minified)} bytes)")
    else:
        print(f"Warning: File not found {abs_path}")

# 2. Compress JS
js_files = [
    r"coremain/www/assets/js/log.js"
]

for rel_path in js_files:
    abs_path = os.path.join(base_dir, rel_path)
    if os.path.exists(abs_path):
        print(f"Processing JS: {rel_path}")
        with open(abs_path, 'r', encoding='utf-8') as f:
            raw = f.read()
        minified = minify_js_safe(raw)
        out_path = abs_path.replace('.js', '.min.js')
        with open(out_path, 'w', encoding='utf-8') as f:
            f.write(minified)
        print(f"  -> Minified to {out_path} ({len(minified)} bytes)")
    else:
        print(f"Warning: File not found {abs_path}")
