#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
CSS 合并和压缩脚本
"""
import os

# CSS 文件路径
css_dir = r'coremain\www\assets\css'
css_files = [
    'log_refactored.css',
    'enhancements.css',
    'performance.css',
    'ui-enhancements.css'
]

# 合并 CSS
bundle_content = []
for css_file in css_files:
    file_path = os.path.join(css_dir, css_file)
    print(f'Reading {css_file}...')
    with open(file_path, 'r', encoding='utf-8') as f:
        content = f.read()
        bundle_content.append(f'/* {css_file} */\n')
        bundle_content.append(content)
        bundle_content.append('\n\n')

# 写入合并文件
bundle_path = os.path.join(css_dir, 'bundle.css')
print(f'Writing bundle.css...')
with open(bundle_path, 'w', encoding='utf-8') as f:
    f.write(''.join(bundle_content))

# 获取文件大小
bundle_size = os.path.getsize(bundle_path)
print(f'✓ Bundle created: {bundle_size:,} bytes')

print('\nNext step: Run postcss to compress')
print('Command: postcss coremain\\www\\assets\\css\\bundle.css --use cssnano -o coremain\\www\\assets\\css\\bundle.min.css')
