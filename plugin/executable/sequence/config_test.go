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

package sequence

import (
	"reflect"
	"testing"
)

func Test_parseExecStr(t *testing.T) {

	tests := []struct {
		name     string
		args     string
		wantTag  string
		wantTyp  string
		wantArgs string
	}{
		{"", " $t1   a 1  ", "t1", "", "a 1"},
		{"", " typ   a 1  ", "", "typ", "a 1"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// [修改] 调用新的 parseExecStr 函数，接收结构体
			got := parseExecStr(tt.args)
			
			// [修改] 验证结构体字段
			if got.Tag != tt.wantTag {
				t.Errorf("parseExecStr() gotTag = %v, want %v", got.Tag, tt.wantTag)
			}
			if got.Type != tt.wantTyp {
				t.Errorf("parseExecStr() gotTyp = %v, want %v", got.Type, tt.wantTyp)
			}
			if got.Args != tt.wantArgs {
				t.Errorf("parseExecStr() gotArgs = %v, want %v", got.Args, tt.wantArgs)
			}
		})
	}
}

func Test_parseMatch(t *testing.T) {
	tests := []struct {
		name string
		args string
		want MatchConfig
	}{
		{"", " $m1  a 1 ", MatchConfig{
			Tag:     "m1",
			Type:    "",
			Args:    "a 1",
			Reverse: false,
		}},
		{"", " ! typ  a 1 ", MatchConfig{
			Tag:     "",
			Type:    "typ",
			Args:    "a 1",
			Reverse: true,
		}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := parseMatch(tt.args); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("parseMatch() = %v, want %v", got, tt.want)
			}
		})
	}
}
