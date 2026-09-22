# AIGateway Console

Vue 3 + TypeScript + Vite 的用户、Key 与插件控制台，直接连接现有 Core 身份接口。UI 使用 shadcn-vue（Reka Nova）官方组件，包括 Sidebar、Table、Dialog、AlertDialog、Select、Tabs、Pagination、Sonner 和 ScrollArea。页面、侧栏、表单弹窗与表格滚动均使用 ScrollArea。

## 启动

需要 Node.js 22.22.2+ 或 24.15+。先按仓库根目录说明启动网关，默认地址 `http://127.0.0.1:8080`。

```sh
cd console
npm ci
npm run dev
```

打开 `http://127.0.0.1:5173/console/`。首次进入，在「首次设置」填写部署时的 `GATEWAY_ADMIN_TOKEN`，创建管理员账号。后续直接用用户名和密码登录。平台令牌只在本次创建请求中发送，不保存在浏览器中。

网关地址不同时，创建 `.env.local`：

```dotenv
GATEWAY_PROXY=http://127.0.0.1:8080
```

代理目标仅供 Vite 开发/预览服务器使用，不会打包到浏览器。浏览器始终访问同源 `/__gateway`。

## 页面与行为

- 用户管理：创建用户，修改管理员角色，重置密码，启用/停用，搜索、筛选和分页。
- Key 管理：管理员查看全部 Key，为用户创建个人 Key，或为平台创建服务 Key，设置有效期和撤销。
- 我的 Key：任何已登录用户管理自己的 Key。Key 明文只展示一次，关闭即清空，不写入持久存储。
- 我的账号：查看身份、角色和创建时间。
- 插件管理：上传 ZIP 安装或替换插件，启停、卸载；安装后自动出现在左侧插件快捷入口。
- 插件工作台：每个插件都有独立页面，查看运行状态、接口和存储绑定，修改环境变量、正文记录选项，通过快捷链接进入 Core 的调用记录、日志、用量页，并自动选中当前插件。Octopus 页面显示当前模型及上游凭证是否配置。
- Core 调用记录：统一入口按插件选择，查看请求状态、耗时、上游调用和正文预览。
- Core 日志：按插件查看实际 console 输出，最新优先，按游标加载更早日志。
- Core 用量：按插件和时间查看逐条 usage 明细，每次上报一行，显示时间、插件、请求 ID 和原始字段；每页 50 条，支持加载更早记录。
- 支持手机导航、浅色/深色切换、加载/空/失败状态。无模拟业务数据。

会话令牌保存在当前标签页的 `sessionStorage`，退出登录清除并请求服务端注销。接口返回 401 时返回登录页。管理员角色、密码修改会使目标用户原会话失效；停用用户也会让其个人 Key 暂停访问。角色限制最终由 Core 接口执行。

用户和 Key 列表最多返回 1,000 条数据，因此筛选、统计和分页基于已加载的记录；达到上限时页面提示。调用记录独立使用每页 50 条的倒序游标分页，正文最多预览 64 KiB。个人和服务 Key 是身份凭证，不能代替管理员登录会话。现有 Codex 插件仍使用自己的旧 Key 配置，尚未迁移。

插件页面仅对管理员开放。配置通过专用管理接口读取，响应不缓存；列表与侧栏接口不返回配置值。无效的环境变量/绑定覆盖会在重新加载前拒绝；有效配置保存会重新加载插件并可能中断在途请求；卸载保留插件存储数据和运行覆盖配置。

当前每个插件使用统一工作台，没有执行插件包内自带的前端代码。部门额度池、计费和模型权限留待后续。

## 构建与部署

```sh
npm run build
npm run preview
```

构建结果在 `dist/`，站点基础路径固定为 `/console/`。预览服务器默认 `http://127.0.0.1:4173/console/`，也会代理到 `GATEWAY_PROXY`。生产环境由现有 Web 服务器托管静态文件；Core 不需要增加静态资源逻辑。

例如将 `dist` 的内容放入 `/srv/aigateway/console/`，在已有 HTTPS Nginx server 中添加：

```nginx
location = /console { return 302 /console/; }
location /console/ {
    root /srv/aigateway;
    try_files $uri $uri/ /console/index.html;
}
location /__gateway/ {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
}
```

刷新任意页面必须回退到 `/console/index.html`。`/__gateway` 必须保留原始路径转发；不要将平台管理令牌写入前端或代理配置。完整 Key 的自动复制需要 HTTPS 或 localhost 的浏览器剪贴板权限，失败时会选中内容供手动复制。

## 验证

```sh
npm run typecheck
npm run build
npx playwright install chromium
npm run test:e2e
```

端到端测试会构建本地 Rust 网关，在临时数据目录启动独立网关（19081）与 Vite（15173），验证真实 API 的首次设置、用户管理、个人/服务 Key、权限、会话失效及手机滚动。测试使用专用的固定测试令牌，运行结束删除临时数据库，不访问现有数据。测试截图位于被 Git 忽略的 `artifacts/`，不会包含完整 Key。

`src/components/ui` 来自官方 shadcn-vue CLI。业务页面位于 `src/views`；API 和会话逻辑位于 `src/lib`。Table 的容器采用官方 ScrollArea + ScrollBar 组合，避免原生表格滚动条。

日志、调用记录和用量的插件下拉框默认选择“全部插件”，也可切换到单个插件。全部视图由服务端统一分页，每行标明所属插件，用量仍然一次上报一行；已卸载插件的保留记录也可查看。
