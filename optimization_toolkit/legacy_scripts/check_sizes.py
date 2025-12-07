#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
文件大小对比脚本
"""
import os

def get_file_size(filepath):
    """获取文件大小（KB）"""
    if os.path.exists(filepath):
        size_bytes = os.path.getsize(filepath)
        size_kb = size_bytes / 1024
        return size_bytes, size_kb
    return 0, 0

def format_size(size_kb):
    """格式化文件大小"""
    return f"{size_kb:.2f} KB"

def calculate_reduction(original, optimized):
    """计算减少百分比"""
    if original == 0:
        return 0
    reduction = ((original - optimized) / original) * 100
    return reduction

print("=" * 80)
print("文件优化效果对比")
print("=" * 80)

# JavaScript 文件
print("\n📦 JavaScript 文件:")
print("-" * 80)

js_original_path = r'coremain\www\assets\js\log.js'
js_old_min_path = r'backup_full_20251207_105541\log.min.js'  # 旧的压缩版本
js_new_min_path = r'coremain\www\assets\js\log.min.js'

js_orig_bytes, js_orig_kb = get_file_size(js_original_path)
js_old_bytes, js_old_kb = get_file_size(js_old_min_path)
js_new_bytes, js_new_kb = get_file_size(js_new_min_path)

print(f"原始文件 (log.js):           {format_size(js_orig_kb):>12} ({js_orig_bytes:,} bytes)")
if js_old_kb > 0:
    old_reduction = calculate_reduction(js_orig_kb, js_old_kb)
    print(f"旧压缩版本 (log.min.js):     {format_size(js_old_kb):>12} ({js_old_bytes:,} bytes) - 减少 {old_reduction:.1f}%")
print(f"新压缩版本 (log.min.js):     {format_size(js_new_kb):>12} ({js_new_bytes:,} bytes) - 减少 {calculate_reduction(js_orig_kb, js_new_kb):.1f}%")

if js_old_kb > 0:
    improvement = calculate_reduction(js_old_kb, js_new_kb)
    print(f"\n✨ 相比旧版本进一步减少:      {format_size(js_old_kb - js_new_kb):>12} ({improvement:.1f}%)")

# CSS 文件
print("\n🎨 CSS 文件:")
print("-" * 80)

css_files = [
    r'coremain\www\assets\css\log_refactored.css',
    r'coremain\www\assets\css\enhancements.css',
    r'coremain\www\assets\css\performance.css',
    r'coremain\www\assets\css\ui-enhancements.css'
]

css_total_bytes = 0
css_total_kb = 0
for css_file in css_files:
    size_bytes, size_kb = get_file_size(css_file)
    css_total_bytes += size_bytes
    css_total_kb += size_kb
    print(f"{os.path.basename(css_file):30} {format_size(size_kb):>12}")

print(f"{'=' * 30} {'=' * 12}")
print(f"{'原始 CSS 总计':30} {format_size(css_total_kb):>12} ({css_total_bytes:,} bytes)")

bundle_path = r'coremain\www\assets\css\bundle.css'
bundle_min_path = r'coremain\www\assets\css\bundle.min.css'

bundle_bytes, bundle_kb = get_file_size(bundle_path)
bundle_min_bytes, bundle_min_kb = get_file_size(bundle_min_path)

print(f"\n合并后 (bundle.css):         {format_size(bundle_kb):>12} ({bundle_bytes:,} bytes)")
print(f"压缩后 (bundle.min.css):     {format_size(bundle_min_kb):>12} ({bundle_min_bytes:,} bytes) - 减少 {calculate_reduction(css_total_kb, bundle_min_kb):.1f}%")

# HTML 文件
print("\n📄 HTML 文件:")
print("-" * 80)

html_original_path = r'coremain\www\log.html'
html_min_path = r'coremain\www\log.min.html'

html_orig_bytes, html_orig_kb = get_file_size(html_original_path)
html_min_bytes, html_min_kb = get_file_size(html_min_path)

print(f"原始文件 (log.html):         {format_size(html_orig_kb):>12} ({html_orig_bytes:,} bytes)")
if html_min_kb > 0:
    print(f"压缩后 (log.min.html):       {format_size(html_min_kb):>12} ({html_min_bytes:,} bytes) - 减少 {calculate_reduction(html_orig_kb, html_min_kb):.1f}%")
else:
    print(f"压缩后 (log.min.html):       处理中...")

# 总计
print("\n" + "=" * 80)
print("📊 总体优化效果:")
print("=" * 80)

original_total_kb = js_orig_kb + css_total_kb + html_orig_kb
optimized_total_kb = js_new_kb + bundle_min_kb + (html_min_kb if html_min_kb > 0 else html_orig_kb)

print(f"\n优化前总大小:  {format_size(original_total_kb):>12} ({int(original_total_kb * 1024):,} bytes)")
print(f"优化后总大小:  {format_size(optimized_total_kb):>12} ({int(optimized_total_kb * 1024):,} bytes)")
print(f"减少大小:      {format_size(original_total_kb - optimized_total_kb):>12}")
print(f"减少比例:      {calculate_reduction(original_total_kb, optimized_total_kb):>11.1f}%")

print("\n" + "=" * 80)
