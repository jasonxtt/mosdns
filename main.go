//go:debug urlstrictcolons=0
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

package main

import (
	"fmt"
	"os"
	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/mlog"
	_ "github.com/IrineSistiana/mosdns/v5/plugin"
	_ "github.com/IrineSistiana/mosdns/v5/tools"
	"github.com/spf13/cobra"
	_ "net/http/pprof"
)

var (
	version      = "v5-ph-srs"
	installMode  bool
	installPort  int
	mainArgsList []string
)

func init() {
	mainArgsList = make([]string, len(os.Args))
	copy(mainArgsList, os.Args)
	
	coremain.SetBuildVersion(version)
	coremain.AddSubCmd(&cobra.Command{
		Use:   "version",
		Short: "Print out version info and exit.",
		Run: func(cmd *cobra.Command, args []string) {
			fmt.Println(version)
		},
	})

	// 添加安装模式命令
	installCmd := &cobra.Command{
		Use:   "install-wizard",
		Short: "Start installation wizard",
		Run: func(cmd *cobra.Command, args []string) {
			coremain.StartInstallWizard(installPort)
		},
	}
	installCmd.Flags().BoolVarP(&installMode, "start", "s", false, "Start installation wizard")
	installCmd.Flags().IntVarP(&installPort, "port", "p", 9098, "Installation wizard port")
	coremain.AddSubCmd(installCmd)
}

func main() {
	// 兼容旧用法：直接运行 mosdns -s
	if len(mainArgsList) > 1 && (mainArgsList[1] == "-s" || mainArgsList[1] == "--start") {
		port := 9098
		for i := 2; i < len(mainArgsList); i++ {
			if mainArgsList[i] == "-p" && i+1 < len(mainArgsList) {
				fmt.Sscanf(mainArgsList[i+1], "%d", &port)
			}
		}
		coremain.StartInstallWizard(port)
		return
	}

	if err := coremain.Run(); err != nil {
		mlog.S().Fatal(err)
	}
}
