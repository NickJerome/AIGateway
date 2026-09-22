import { createRouter, createWebHistory } from 'vue-router'
import { session } from './lib/session'

export const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: '/login', name: 'login', component: () => import('./views/LoginView.vue') },
    {
      path: '/',
      component: () => import('./layouts/AppLayout.vue'),
      children: [
        {
          path: 'plugins',
          component: () => import('./views/PluginsView.vue'),
          meta: { title: '插件管理', admin: true },
        },
        {
          path: 'plugins/:name',
          component: () => import('./views/PluginView.vue'),
          props: true,
          meta: { title: '插件工作台', admin: true },
        },
        ...(['traces', 'logs', 'usage'] as const).map((kind) => ({
          path: kind,
          component: () => import('./views/TelemetryView.vue'),
          props: { kind },
          meta: { title: { traces: '调用记录', logs: '日志', usage: '用量' }[kind], admin: true },
        })),
        { path: '', component: { render: () => null } },
        {
          path: 'users',
          component: () => import('./views/UsersView.vue'),
          meta: { title: '用户管理', admin: true },
        },
        {
          path: 'keys',
          component: () => import('./views/KeysView.vue'),
          props: { scope: 'all' },
          meta: { title: 'Key 管理', admin: true },
        },
        {
          path: 'my-keys',
          component: () => import('./views/KeysView.vue'),
          props: { scope: 'mine' },
          meta: { title: '我的 Key' },
        },
        {
          path: 'account',
          component: () => import('./views/AccountView.vue'),
          meta: { title: '我的账号' },
        },
      ],
    },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
})

router.beforeEach(async (to) => {
  await session.restore()
  if (to.name === 'login') {
    if (session.user.value) return '/'
  } else {
    if (!session.user.value) return { name: 'login', query: { redirect: to.fullPath } }
    if (to.path === '/') return session.isAdmin.value ? '/users' : '/my-keys'
    if (to.meta.admin && !session.isAdmin.value) return '/my-keys'
  }
})

router.afterEach((to) => {
  document.title = `${to.meta.title ?? '登录'} · AIGateway`
})

window.addEventListener('gateway:session-expired', () => {
  session.clear('登录已过期或账号状态已变更，请重新登录。')
  void router.replace('/login')
})
