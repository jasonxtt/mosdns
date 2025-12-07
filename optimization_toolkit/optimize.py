#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Mosdns 前端资源优化工具
用于压缩和解压缩 JavaScript、CSS 和 HTML 文件
"""

import os
import sys
import re
import shutil
import subprocess
from datetime import datetime

# 配置
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(SCRIPT_DIR)
WEB_DIR = os.path.join(PROJECT_ROOT, "coremain/www")
ASSETS_JS_DIR = f"{WEB_DIR}/assets/js"
ASSETS_CSS_DIR = f"{WEB_DIR}/assets/css"

# CSS 文件列表
CSS_FILES = [
    "log_refactored.css",
    "enhancements.css",
    "performance.css",
    "ui-enhancements.css"
]

def print_header(text):
    """打印标题"""
    print("\n" + "=" * 60)
    print(f"  {text}")
    print("=" * 60 + "\n")

def print_success(text):
    """打印成功消息"""
    print(f"✓ {text}")

def print_error(text):
    """打印错误消息"""
    print(f"✗ {text}")

def print_info(text):
    """打印信息"""
    print(f"→ {text}")

def minify_css(css_content):
    """简单的 CSS 压缩"""
    # 移除注释
    css_content = re.sub(r'/\*[\s\S]*?\*/', '', css_content)
    
    # 移除多余的空白
    css_content = re.sub(r'\s+', ' ', css_content)
    
    # 移除 { } : ; , 周围的空格
    css_content = re.sub(r'\s*{\s*', '{', css_content)
    css_content = re.sub(r'\s*}\s*', '}', css_content)
    css_content = re.sub(r'\s*:\s*', ':', css_content)
    css_content = re.sub(r'\s*;\s*', ';', css_content)
    css_content = re.sub(r'\s*,\s*', ',', css_content)
    
    # 移除最后一个分号（在 } 之前）
    css_content = re.sub(r';}', '}', css_content)
    
    return css_content.strip()

def compress_resources():
    """压缩资源文件"""
    print_header("开始压缩资源文件")
    
    # 1. 压缩 JavaScript
    print_info("步骤 1/3: 压缩 JavaScript...")
    js_input = f"{ASSETS_JS_DIR}/log.js"
    js_output = f"{ASSETS_JS_DIR}/log.min.js"
    
    if not os.path.exists(js_input):
        print_error(f"找不到文件: {js_input}")
        return False
    
    try:
        # 使用 Terser 压缩
        cmd = f'terser {js_input} --compress --mangle --output {js_output} --source-map "url=log.min.js.map"'
        result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
        
        if result.returncode == 0:
            original_size = os.path.getsize(js_input)
            compressed_size = os.path.getsize(js_output)
            reduction = ((original_size - compressed_size) / original_size) * 100
            print_success(f"JavaScript 压缩完成: {original_size:,} → {compressed_size:,} bytes ({reduction:.1f}% 减少)")
        else:
            print_error(f"JavaScript 压缩失败: {result.stderr}")
            return False
    except Exception as e:
        print_error(f"JavaScript 压缩出错: {e}")
        return False
    
    # 2. 合并并压缩 CSS
    print_info("步骤 2/3: 合并并压缩 CSS...")
    
    # 合并 CSS
    bundle_content = []
    total_original_size = 0
    
    for css_file in CSS_FILES:
        css_path = f"{ASSETS_CSS_DIR}/{css_file}"
        if os.path.exists(css_path):
            with open(css_path, 'r', encoding='utf-8') as f:
                content = f.read()
                total_original_size += len(content)
                bundle_content.append(f'/* {css_file} */\n')
                bundle_content.append(content)
                bundle_content.append('\n\n')
            print_info(f"  已合并: {css_file}")
        else:
            print_error(f"  找不到文件: {css_path}")
    
    # 写入合并文件
    bundle_path = f"{ASSETS_CSS_DIR}/bundle.css"
    with open(bundle_path, 'w', encoding='utf-8') as f:
        f.write(''.join(bundle_content))
    
    # 压缩 CSS
    minified_css = minify_css(''.join(bundle_content))
    bundle_min_path = f"{ASSETS_CSS_DIR}/bundle.min.css"
    
    with open(bundle_min_path, 'w', encoding='utf-8') as f:
        f.write(minified_css)
    
    compressed_size = len(minified_css)
    reduction = ((total_original_size - compressed_size) / total_original_size) * 100
    print_success(f"CSS 压缩完成: {total_original_size:,} → {compressed_size:,} bytes ({reduction:.1f}% 减少)")
    
    # 3. 压缩 HTML
    print_info("步骤 3/3: 压缩 HTML...")
    html_input = f"{WEB_DIR}/log.html"
    html_output = f"{WEB_DIR}/log.min.html"
    
    if not os.path.exists(html_input):
        print_error(f"找不到文件: {html_input}")
        return False
    
    try:
        cmd = f'html-minifier-terser --collapse-whitespace --remove-comments --remove-optional-tags --remove-redundant-attributes --remove-script-type-attributes --remove-tag-whitespace --use-short-doctype --minify-css true --minify-js true -o {html_output} {html_input}'
        result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
        
        if result.returncode == 0:
            original_size = os.path.getsize(html_input)
            compressed_size = os.path.getsize(html_output)
            reduction = ((original_size - compressed_size) / original_size) * 100
            print_success(f"HTML 压缩完成: {original_size:,} → {compressed_size:,} bytes ({reduction:.1f}% 减少)")
        else:
            print_error(f"HTML 压缩失败: {result.stderr}")
            return False
    except Exception as e:
        print_error(f"HTML 压缩出错: {e}")
        return False
    
    print_header("压缩完成！")
    print_info("提示: 压缩后的文件已生成，HTML 文件已自动引用压缩版本")
    return True

def restore_resources():
    """恢复到未压缩状态（开发模式）"""
    print_header("恢复到开发模式")
    
    print_info("此操作将删除压缩文件，恢复到开发模式...")
    print_info("压缩文件将被删除:")
    print_info(f"  - {ASSETS_JS_DIR}/log.min.js")
    print_info(f"  - {ASSETS_CSS_DIR}/bundle.css")
    print_info(f"  - {ASSETS_CSS_DIR}/bundle.min.css")
    print_info(f"  - {WEB_DIR}/log.min.html")
    
    confirm = input("\n确认继续? (y/N): ")
    if confirm.lower() != 'y':
        print_info("操作已取消")
        return False
    
    # 删除压缩文件
    files_to_remove = [
        f"{ASSETS_JS_DIR}/log.min.js",
        f"{ASSETS_JS_DIR}/log.min.js.map",
        f"{ASSETS_CSS_DIR}/bundle.css",
        f"{ASSETS_CSS_DIR}/bundle.min.css",
        f"{WEB_DIR}/log.min.html"
    ]
    
    for file_path in files_to_remove:
        if os.path.exists(file_path):
            os.remove(file_path)
            print_success(f"已删除: {file_path}")
    
    print_header("恢复完成！")
    print_info("提示: 开发时请确保 HTML 引用的是未压缩版本的文件")
    return True

def show_status():
    """显示当前状态"""
    print_header("资源文件状态")
    
    # 检查压缩文件是否存在
    compressed_files = {
        "JavaScript (压缩)": f"{ASSETS_JS_DIR}/log.min.js",
        "CSS (合并)": f"{ASSETS_CSS_DIR}/bundle.css",
        "CSS (压缩)": f"{ASSETS_CSS_DIR}/bundle.min.css",
        "HTML (压缩)": f"{WEB_DIR}/log.min.html"
    }
    
    print("压缩文件状态:")
    for name, path in compressed_files.items():
        if os.path.exists(path):
            size = os.path.getsize(path)
            print_success(f"{name}: 存在 ({size:,} bytes)")
        else:
            print_error(f"{name}: 不存在")
    
    print("\n原始文件:")
    original_files = {
        "JavaScript": f"{ASSETS_JS_DIR}/log.js",
        "HTML": f"{WEB_DIR}/log.html"
    }
    
    for name, path in original_files.items():
        if os.path.exists(path):
            size = os.path.getsize(path)
            print_success(f"{name}: 存在 ({size:,} bytes)")
        else:
            print_error(f"{name}: 不存在")
    
    print("\nCSS 源文件:")
    for css_file in CSS_FILES:
        css_path = f"{ASSETS_CSS_DIR}/{css_file}"
        if os.path.exists(css_path):
            size = os.path.getsize(css_path)
            print_success(f"{css_file}: 存在 ({size:,} bytes)")
        else:
            print_error(f"{css_file}: 不存在")

def show_help():
    """显示帮助信息"""
    print_header("Mosdns 资源优化工具")
    print("用法: python optimize.py [命令]")
    print("\n可用命令:")
    print("  compress    压缩资源文件 (JS, CSS, HTML)")
    print("  restore     恢复到开发模式 (删除压缩文件)")
    print("  status      显示当前资源文件状态")
    print("  help        显示此帮助信息")
    print("\n示例:")
    print("  python optimize.py compress    # 压缩所有资源")
    print("  python optimize.py status      # 查看状态")
    print("  python optimize.py restore     # 恢复开发模式")
    print("\n详细文档请查看: OPTIMIZATION.md")

def main():
    """主函数"""
    if len(sys.argv) < 2:
        show_help()
        return
    
    command = sys.argv[1].lower()
    
    if command == "compress":
        compress_resources()
    elif command == "restore":
        restore_resources()
    elif command == "status":
        show_status()
    elif command == "help":
        show_help()
    else:
        print_error(f"未知命令: {command}")
        print_info("使用 'python optimize.py help' 查看帮助")

if __name__ == "__main__":
    main()
