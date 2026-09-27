# Representative chain and independent expectations — revision 2, not executed

## Source and deliberate reduction

来源为本地配置包 config_lite_all/config_custom.yaml 的 include、qtype/reject、$sequence_main/$sequence_other，以及 sub_config/forward_nocn.yaml 的 cache→upstream 子链。它是“同类实际组合”的裁剪，不是原文件原样兼容证明。

保留外部语法和关键结构；移除本链无关的 UI/API、switch、special_groups、flow_setter、cname/ecs/lazy/dump。为隔离验证，将 aliapi 多公网加密上游替换为已有 forward 的受控 UDP/TCP peer；真实 aliapi/策略/协议保留后续归属。正整数 cache size 用小容量，不据此推断生产规则规模或性能。rev1 两条手写图不再是主验收。

目录：config.yaml、sub_config/routes.yaml、rules/local.txt。配置路径/端口均是文本示例，执行时替换实际独占临时目录和空闲端口，不能要求 loader 支持模板变量。规则文件使用绝对路径，另用定向测试核对现有相对路径约定。

## config.yaml

~~~yaml
log:
  level: error
include:
  - sub_config/routes.yaml
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qtype 65
        exec: reject 0
      - matches: qname $blocked
        exec: reject 3
      - exec: $sequence_routed
      - matches: has_resp
        exec: accept
      - exec: $sequence_default
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:26353"
      enable_audit: true
~~~

## sub_config/routes.yaml

~~~yaml
plugins:
  - tag: sequence_routed
    type: sequence
    args:
      - matches: qname $local_domains
        exec: $sequence_local
  - tag: sequence_local
    type: sequence
    args:
      - exec: $cache_main
      - exec: $local_forward
  - tag: sequence_default
    type: sequence
    args:
      - exec: $default_forward
  - tag: cache_main
    type: cache
    args:
      size: 64
      lazy_cache_ttl: 0
  - tag: local_forward
    type: forward
    args:
      upstreams:
        - tag: local_peer
          addr: "udp://127.0.0.1:26361"
  - tag: default_forward
    type: forward
    args:
      upstreams:
        - tag: default_peer
          addr: "tcp://127.0.0.1:26362"
  - tag: blocked
    type: domain_set
    args:
      exps:
        - full:blocked.test
        - full:another-blocked.test
  - tag: local_domains
    type: domain_set
    args:
      files:
        - "/tmp/phase5b-fixture/rules/local.txt"
~~~

rules/local.txt：

~~~text
domain:local.test
full:local.only.test
~~~

local_peer 返回 192.0.2.21，default_peer 返回 192.0.2.22；返回请求 question/ID、一条 A/TTL=60。用受控时钟测试 TTL；公网随机耗时不作为 oracle。

## Primary oracle

| 请求/条件 | 本次 peer 增量 | 最终 DNS | 需要证明 |
| --- | --- | --- | --- |
| blocked.test A | 两者 0 | NXDOMAIN/RCODE=3，无答案 | 拒绝不进入缓存/上游 |
| other.test HTTPS | 两者 0 | NOERROR/RCODE=0，无答案 | qtype/reject 0 不误转发 |
| a.local.test A 首次 | local +1，default 0 | 192.0.2.21 | file suffix 命中、两层 direct child、cache miss |
| 同名再次、不同 ID | 两者 0 | 同地址、当前 ID、正确递减 TTL | cache hit 结束 child 后继，返回父；不泄露 default |
| local.only.test A 首次 | local +1，default 0 | 192.0.2.21 | 多条 file rule 的 exact 匹配 |
| other.test A | local 0，default +1 | 192.0.2.22 | routed 无响应时父继续 default |

成功分支最终实际 matcher/exec 在 sequence_main，因此 named final_sequence=sequence_main；上游 source 分别为 local_peer/default_peer，hit 为真实 cache source；attempts 只列实际网络调用。block 是本地来源。完整 flow_setter/effective_tag 产品语义不因该子集 oracle 宣称完成。

## Small variants, not a new matrix

- 同链换 TCP listener（沿原 idle_timeout 契约）、audit off/on：wire/peer 顺序不变，不硬编码所有参数笛卡尔积。
- 在 child 后父添加另一合法 forward：child cache 保留其后继响应，父最终响应可替换；再查询该 cache 必须仍是 child 地址。另把 cache 移 entry 并删除 child cache，证明 entry 包裹子调用的后继完成点。
- child accept/reject 后父执行一条可观测后继；child exit 时父后继不执行；try child exit 时继续，普通错误仍 SERVFAIL。jump/goto/return/exec list 沿既有 scope 契约定向补例。
- 慢 peer/坏 question/坏包/取消、后续 forward 失败：终态正确、未执行的 peer 零增量、无坏/残留缓存写入；实际错误位置可观测。
- 第二次 cache 访问明确受控失败；fuel 循环有界；旧 W1/W2/W3 原样回归。
- missing include/rule file、子 include、坏规则/重复 tag/跨类型 ref/reject>15 在 bind/查询前报路径错误；重排定义不改变有效分支。

预期来自配置语义、既有模块契约和真实 peer，不从候选输出生成 golden。以上为规划文本，未运行。
