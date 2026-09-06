<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue';
import {
    LuCopy,
    LuDownload,
    LuEye,
    LuEyeOff,
    LuLoader2,
    LuShare2,
    LuX,
} from 'vue-icons-plus/lu';
import { snapdom } from '@zumer/snapdom';
import dayjs from 'dayjs';
import { toast } from '@/composables/toast';
import { useTranslations } from '@/composables/i18n';
import { buttonVariants } from '@/components/ui/button';
import Logo from '@/components/modules/logo/Logo.vue';
import HomeSections from './home-sections.vue';
import { homeViewStore } from './store';

// HomeActions 向稳定顶栏提供渠道名模糊开关和分享入口, 并承载分享图舞台与预览。
const { t } = useTranslations('toolbar');
const { t: tCommon } = useTranslations('common');
const isStaged = ref(false); // 为真时屏外挂载正文副本供截图, 期间分享按钮转为加载态。
const preview = ref<{ url: string; blob: Blob } | null>(null); // url 用于展示, blob 用于复制和下载。
const stageRef = ref<HTMLDivElement | null>(null); // 屏外副本的根节点, 截图取材于此。

// 预览关闭或组件卸载时释放临时对象 URL。
watch(preview, (newVal, oldVal) => {
    if (oldVal?.url) URL.revokeObjectURL(oldVal.url);
});

onBeforeUnmount(() => {
    if (preview.value?.url) URL.revokeObjectURL(preview.value.url);
});

// 屏外副本在自身树内按固定宽度布局, 图表和栅格都不受当前窗口宽度影响; 等其动画结束后截图。
watch(isStaged, (staged, _, onCleanup) => {
    if (!staged) return;

    let cancelled = false;
    const timer = window.setTimeout(async () => {
        if (cancelled || !stageRef.value) return;
        try {
            const blob = await snapdom.toBlob(stageRef.value, { type: 'png', scale: 2, embedFonts: true });
            if (!cancelled) {
                preview.value = { url: URL.createObjectURL(blob), blob };
            }
        } catch (error) {
            if (!cancelled) {
                toast.error(error instanceof Error ? error.message : String(error));
            }
        } finally {
            if (!cancelled) {
                isStaged.value = false;
            }
        }
        // snapdom 只序列化当前 DOM, 不推进动画, 故等副本自身动画跑完: 趋势图 1500ms, 数字 800ms, 余量 300ms。
    }, 1800);

    onCleanup(() => {
        cancelled = true;
        window.clearTimeout(timer);
    });
});

const handleCopyImage = async () => {
    if (!preview.value) return;
    try {
        await navigator.clipboard.write([new ClipboardItem({ 'image/png': preview.value.blob })]);
        toast.success(tCommon('copy.success'));
    } catch {
        toast.error(tCommon('copy.failed'));
    }
};

const handleDownloadImage = () => {
    if (!preview.value) return;
    const link = document.createElement('a');
    link.href = preview.value.url;
    link.download = `octopus-${dayjs().format('YYYYMMDD')}.png`;
    link.click();
};
</script>

<template>
    <div class="flex items-center gap-2">
        <button
            type="button"
            @click="homeViewStore.setChannelNameHidden(!homeViewStore.isChannelNameHidden)"
            :aria-pressed="homeViewStore.isChannelNameHidden"
            :aria-label="t('hideChannelName')"
            :class="buttonVariants({
                variant: 'ghost',
                size: 'icon',
                className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground',
            })"
        >
            <LuEyeOff v-if="homeViewStore.isChannelNameHidden" class="size-4 transition-colors duration-300" />
            <LuEye v-else class="size-4 transition-colors duration-300" />
        </button>

        <button
            type="button"
            @click="isStaged = true"
            :disabled="isStaged"
            :aria-label="t('share')"
            :class="buttonVariants({
                variant: 'ghost',
                size: 'icon',
                className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground disabled:opacity-50',
            })"
        >
            <LuLoader2 v-if="isStaged" class="size-4 animate-spin" />
            <LuShare2 v-else class="size-4 transition-colors duration-300" />
        </button>

        <div
            v-if="isStaged"
            ref="stageRef"
            aria-hidden="true"
            class="fixed top-0 rounded-3xl bg-background p-4 text-foreground"
            style="left: -10000px; box-sizing: content-box; width: 1002px;"
        >
            <!-- 分享图抬头, 与应用顶栏一致的标识和名称。 -->
            <div class="mb-4 flex items-center gap-x-2 px-2">
                <Logo :size="48" />
                <span class="text-3xl font-bold">Octopus</span>
            </div>
            <HomeSections />
        </div>

        <div
            v-if="preview"
            class="fixed inset-0 z-60 flex flex-col items-center justify-center gap-3 bg-background/95 p-4 backdrop-blur-sm"
        >
            <img
                :src="preview.url"
                alt=""
                class="max-h-[75vh] min-h-0 w-auto max-w-full rounded-2xl border border-border object-contain"
            />
            <div class="flex shrink-0 items-center gap-2">
                <button
                    type="button"
                    @click="handleCopyImage"
                    class="flex size-10 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors hover:text-foreground active:scale-95"
                >
                    <LuCopy class="size-4" />
                </button>
                <button
                    type="button"
                    @click="handleDownloadImage"
                    class="flex size-10 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors hover:text-foreground active:scale-95"
                >
                    <LuDownload class="size-4" />
                </button>
                <button
                    type="button"
                    @click="preview = null"
                    class="flex size-10 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors hover:text-foreground active:scale-95"
                >
                    <LuX class="size-4" />
                </button>
            </div>
        </div>
    </div>
</template>
