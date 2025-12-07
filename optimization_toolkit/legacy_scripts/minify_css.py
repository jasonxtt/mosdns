#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
简单的 CSS 压缩脚本
移除注释、空白和换行
"""
import re

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

# 读取合并的 CSS
input_file = r'coremain\www\assets\css\bundle.css'
output_file = r'coremain\www\assets\css\bundle.min.css'

print(f'Reading {input_file}...')
with open(input_file, 'r', encoding='utf-8') as f:
    css_content = f.read()

original_size = len(css_content)
print(f'Original size: {original_size:,} bytes')

print('Minifying CSS...')
minified_css = minify_css(css_content)

minified_size = len(minified_css)
reduction = ((original_size - minified_size) / original_size) * 100

print(f'Writing {output_file}...')
with open(output_file, 'w', encoding='utf-8') as f:
    f.write(minified_css)

print(f'\n✓ CSS minified successfully!')
print(f'  Original:  {original_size:,} bytes')
print(f'  Minified:  {minified_size:,} bytes')
print(f'  Reduction: {reduction:.1f}%')
