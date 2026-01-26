/*
 * Copyright (C) 2020-2022, IrineSistiana
 *
 * This file is part of mosdns.
 *
 * mosdns is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * mosdns is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package data_provider

import (
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
)

type DomainMatcherProvider interface {
	GetDomainMatcher() domain.Matcher[struct{}]
}

type IPMatcherProvider interface {
	GetIPMatcher() netlist.Matcher
}

// RuleExporter 是新增加的接口，允许插件导出其内部的文本规则列表，并支持变更通知。
// 这允许 domain_mapper 插件聚合其他插件的规则。
type RuleExporter interface {
	// GetRules 返回当前生效的所有规则字符串（如 "full:google.com", "regexp:.*"）
	GetRules() ([]string, error)
	// Subscribe 注册一个回调函数，当规则集发生变化（文件更新、API上传等）时调用
	Subscribe(callback func())
}
