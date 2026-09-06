<script setup lang="ts">
import { defineAsyncComponent, watchEffect } from 'vue';
import { useQueryClient } from '@tanstack/vue-query';
import { AnimatePresence, motion } from 'motion-v';
import { Toaster } from '@/components/ui/toaster';
import { authStore } from '@/stores/auth';
import { appStore } from '@/stores/app';
import { pageImports } from '@/lib/page-preload';
import AppShell from '@/components/AppShell.vue';
import {
    apiKeyDashboardStatsQueryOptions,
    apiKeyListQueryOptions,
    channelListQueryOptions,
    groupListQueryOptions,
    modelListQueryOptions,
    statsDailyQueryOptions,
    statsHourlyQueryOptions,
    statsTotalQueryOptions,
} from '@/api/queries';

// 页面与顶栏操作共用 pageImports 中的懒加载模块。
const Home = defineAsyncComponent(() => pageImports.home().then((m) => m.Home));
const Channel = defineAsyncComponent(() => pageImports.channel().then((m) => m.Channel));
const Group = defineAsyncComponent(() => pageImports.group().then((m) => m.Group));
const Model = defineAsyncComponent(() => pageImports.model().then((m) => m.Model));
const Log = defineAsyncComponent(() => pageImports.log().then((m) => m.Log));
const Setting = defineAsyncComponent(() => pageImports.setting().then((m) => m.Setting));
const HomeActions = defineAsyncComponent(() => pageImports.home().then((m) => m.HomeActions));
const ChannelActions = defineAsyncComponent(() => pageImports.channel().then((m) => m.ChannelActions));
const GroupActions = defineAsyncComponent(() => pageImports.group().then((m) => m.GroupActions));
const ModelActions = defineAsyncComponent(() => pageImports.model().then((m) => m.ModelActions));

const LoginForm = defineAsyncComponent(() => import('@/components/modules/login/LoginForm.vue'));
const APIKeyDashboard = defineAsyncComponent(() => import('@/components/modules/apikey-dashboard/ApiKeyDashboard.vue'));

const queryClient = useQueryClient();

// 认证就绪后预取首屏数据（对齐 React 版 useEffect 预取）。
watchEffect(() => {
    if (authStore.isLoading || !authStore.isAuthenticated) return;

    if (authStore.isAPIKeyAuth) {
        void queryClient.prefetchQuery(apiKeyDashboardStatsQueryOptions);
    } else {
        void queryClient.prefetchQuery(apiKeyListQueryOptions);
        void queryClient.prefetchQuery(channelListQueryOptions);
        void queryClient.prefetchQuery(groupListQueryOptions);
        void queryClient.prefetchQuery(modelListQueryOptions);
        void queryClient.prefetchQuery(statsDailyQueryOptions);
        void queryClient.prefetchQuery(statsHourlyQueryOptions);
        void queryClient.prefetchQuery(statsTotalQueryOptions);
    }
});

// 当前界面提交后淡出并移除 HTML 首屏加载动画（对应 React 版 InitialLoadingGate）。
watchEffect((onCleanup) => {
    if (authStore.isLoading) return;
    const loader = document.getElementById('initial-loader');
    if (!loader || loader.dataset.state === 'hidden') return;
    onCleanup(() => {});

    loader.dataset.state = 'hidden';
    loader.classList.add('octo-hide');
    window.setTimeout(() => loader.remove(), 220);
});
</script>

<template>
    <Toaster position="top-left" />

    <template v-if="authStore.isLoading">
        <!-- 首屏 loading 仍由 #initial-loader 呈现 -->
    </template>

    <!-- 登录页面 -->
    <template v-else-if="!authStore.isAuthenticated">
        <LoginForm />
    </template>

    <!-- API Key 仪表盘 -->
    <template v-else-if="authStore.isAPIKeyAuth">
        <APIKeyDashboard />
    </template>

    <!-- 普通用户应用 -->
    <AppShell v-else>
        <template #actions>
            <HomeActions v-if="appStore.currentPage === 'home'" />
            <ChannelActions v-else-if="appStore.currentPage === 'channel'" />
            <GroupActions v-else-if="appStore.currentPage === 'group'" />
            <ModelActions v-else-if="appStore.currentPage === 'model'" />
        </template>

        <AnimatePresence mode="sync">
            <motion.div
                :key="appStore.currentPage"
                :initial="{ opacity: 0, scale: 0.8 }"
                :animate="{
                    opacity: 1,
                    scale: 1,
                    transition: { duration: 0.5, ease: [0.16, 1, 0.3, 1], delay: 0.1 },
                }"
                :exit="{ opacity: 0, scale: 0.98, transition: { duration: 0.25 } }"
                class="absolute inset-0 min-h-0 overflow-hidden"
            >
                <Home v-if="appStore.currentPage === 'home'" />
                <Channel v-else-if="appStore.currentPage === 'channel'" />
                <Group v-else-if="appStore.currentPage === 'group'" />
                <Model v-else-if="appStore.currentPage === 'model'" />
                <Log v-else-if="appStore.currentPage === 'log'" />
                <Setting v-else-if="appStore.currentPage === 'setting'" />
            </motion.div>
        </AnimatePresence>
    </AppShell>
</template>
