<script setup lang="ts">
import { useTranslations } from '@/composables/i18n';
import PageActions from '@/components/common/PageActions.vue';
import {
    MorphingDialogClose,
    MorphingDialogTitle,
    MorphingDialogDescription,
} from '@/components/ui/morphing-dialog';
import ChannelForm from './Form.vue';

// ChannelActions 向稳定顶栏提供渠道页面的搜索、视图选项和创建入口。
const { t: tCreate } = useTranslations('channel.create');
</script>

<template>
    <PageActions page="channel">
        <div class="w-screen max-w-full md:max-w-xl h-full min-h-0 flex flex-col">
            <MorphingDialogTitle class="shrink-0">
                <header class="mb-6 flex items-center justify-between">
                    <h2 class="text-2xl font-bold text-card-foreground">{{ tCreate('dialogTitle') }}</h2>
                    <MorphingDialogClose
                        class="relative right-0 top-0"
                        :variants="{
                            initial: { opacity: 0, scale: 0.8 },
                            animate: { opacity: 1, scale: 1 },
                            exit: { opacity: 0, scale: 0.8 },
                        }"
                    />
                </header>
            </MorphingDialogTitle>
            <MorphingDialogDescription :disable-layout-animation="true" class="flex-1 min-h-0 overflow-auto">
                <ChannelForm />
            </MorphingDialogDescription>
        </div>
    </PageActions>
</template>
