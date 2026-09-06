<script setup lang="ts">
import { ref, watch } from 'vue';
import { NAV_ITEMS, appStore, setCurrentPage, type Page } from '@/stores/app';
import { preloadPage } from '@/lib/page-preload';
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';

// AppShell 作为普通用户界面的稳定布局层，统一渲染导航、顶栏和页面内容。
const { t } = useTranslations('navbar');

const activeOffsetIndex = ref(Math.max(0, NAV_ITEMS.findIndex((route) => route.id === appStore.currentPage))); // activeOffsetIndex 表示选中项在 Dock 中的位置。
const hoveredIndex = ref<number | null>(null); // hoveredIndex 表示当前悬浮项的位置。
const isNavHovered = ref(false); // isNavHovered 表示悬浮背景是否显示。
const hoverIndicatorRef = ref<HTMLElement | null>(null); // hoverIndicatorRef 用于在淡入前确认悬浮背景的新位置。

// currentPage 变化时重新计算指示器位置。
watch(
    () => appStore.currentPage,
    (page) => {
        activeOffsetIndex.value = Math.max(0, NAV_ITEMS.findIndex((route) => route.id === page));
    },
);

function onNavEnter(route: Page, index: number) {
    // 首次进入先在不可见状态下定位，避免背景从上一次位置移动过来。
    if (!isNavHovered.value) {
        hoveredIndex.value = index;
        requestAnimationFrame(() => {
            hoverIndicatorRef.value?.getBoundingClientRect();
            isNavHovered.value = true;
        });
    } else {
        hoveredIndex.value = index;
    }
    preloadPage(route);
}
</script>

<template>
    <div
        class="mx-auto flex h-dvh max-w-6xl animate-in flex-col overflow-hidden px-3 fade-in duration-300 md:grid md:grid-cols-[auto_1fr] md:grid-rows-[auto_minmax(0,1fr)] md:gap-x-6 md:px-6"
    >
        <div class="relative z-50 md:row-span-2 md:min-h-screen">
            <nav
                aria-label="Main Navigation"
                :class="
                    cn(
                        'fixed bottom-6 left-1/2 isolate -translate-x-1/2 flex animate-in items-center gap-1 p-3 fade-in zoom-in-95 duration-300',
                        'md:sticky md:top-30 md:left-auto md:bottom-auto md:translate-x-0 md:flex-col md:gap-3',
                        'bg-sidebar text-sidebar-foreground border border-sidebar-border rounded-3xl',
                    )
                "
                @mouseleave="isNavHovered = false"
            >
                <span
                    aria-hidden="true"
                    class="pointer-events-none absolute left-3 top-3 z-10 size-10 rounded-2xl bg-sidebar-primary transition-transform duration-300 ease-out [transform:translateX(var(--nav-offset-x))] md:size-12 md:[transform:translateY(var(--nav-offset-y))]"
                    :style="{
                        '--nav-offset-x': `${activeOffsetIndex * 2.75}rem`,
                        '--nav-offset-y': `${activeOffsetIndex * 3.75}rem`,
                    }"
                />
                <span
                    ref="hoverIndicatorRef"
                    aria-hidden="true"
                    :class="
                        cn(
                            'pointer-events-none absolute left-3 top-3 z-0 size-10 [transform:translateX(var(--nav-offset-x))] md:size-12 md:[transform:translateY(var(--nav-offset-y))]',
                            isNavHovered ? 'transition-transform duration-300 ease-out' : 'transition-none',
                        )
                    "
                    :style="{
                        '--nav-offset-x': `${(hoveredIndex ?? activeOffsetIndex) * 2.75}rem`,
                        '--nav-offset-y': `${(hoveredIndex ?? activeOffsetIndex) * 3.75}rem`,
                    }"
                >
                    <span
                        class="absolute inset-0 rounded-2xl bg-sidebar-accent transition-opacity duration-350 ease-linear"
                        :style="{ opacity: isNavHovered ? 1 : 0 }"
                    />
                </span>
                <button
                    v-for="(route, index) in NAV_ITEMS"
                    :key="route.id"
                    type="button"
                    :aria-label="route.label"
                    :aria-current="appStore.currentPage === route.id ? 'page' : undefined"
                    @mouseenter="onNavEnter(route.id, index)"
                    @focus="preloadPage(route.id)"
                    @touchstart="preloadPage(route.id)"
                    @click="
                        preloadPage(route.id);
                        setCurrentPage(route.id);
                    "
                    :class="
                        cn(
                            'relative z-20 flex size-10 items-center justify-center rounded-2xl p-2 transition-[color,scale] duration-150 ease-linear hover:z-30 hover:scale-110 active:scale-95 md:size-12 md:p-3',
                            appStore.currentPage === route.id
                                ? 'text-sidebar-primary-foreground'
                                : 'text-sidebar-foreground/60',
                        )
                    "
                >
                    <span class="relative z-10">
                        <component :is="route.icon" :size="24" :stroke-width="2" />
                    </span>
                </button>
            </nav>
        </div>

        <header class="my-3 md:my-6 flex flex-none items-center gap-x-2 px-2">
            <div class="min-w-0 flex-1 overflow-hidden">
                <Transition name="page-title" mode="out-in">
                    <div :key="appStore.currentPage" class="flex items-center">
                        <span class="mt-1 truncate text-3xl font-bold">
                            {{ t(appStore.currentPage) }}
                        </span>
                    </div>
                </Transition>
            </div>
            <div v-if="$slots.actions" class="ml-auto">
                <slot name="actions" />
            </div>
        </header>

        <main class="relative flex min-h-0 w-full min-w-0 flex-1 flex-col overflow-hidden">
            <slot />
        </main>
    </div>
</template>

<style scoped>
/* 页面标题按导航方向滑动（对应 React 版 variants 的 ±32px 位移）。 */
.page-title-enter-active,
.page-title-leave-active {
    transition:
        transform 0.3s ease,
        opacity 0.3s ease;
}
.page-title-enter-from {
    transform: translateY(calc(32px * v-bind('appStore.direction')));
    opacity: 0;
}
.page-title-leave-to {
    transform: translateY(calc(-32px * v-bind('appStore.direction')));
    opacity: 0;
}
</style>
