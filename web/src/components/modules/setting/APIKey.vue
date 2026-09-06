<script setup lang="ts">
import { LuMaximize2 } from 'vue-icons-plus/lu';
import {
    MorphingDialog,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogTrigger,
} from '@/components/ui/morphing-dialog';
import APIKeyPanel from './APIKeyPanel.vue';
import APIKeyDialogPanel from './APIKeyDialogPanel.vue';
</script>

<template>
    <APIKeyPanel
        id-prefix="apikey"
        container-class-name="rounded-3xl border border-border bg-card p-6 space-y-5 relative"
        list-class-name="space-y-2 h-36 overflow-y-auto"
    >
        <template #headerExtra="{ disabled }">
            <MorphingDialog>
                <button
                    v-if="disabled"
                    type="button"
                    disabled
                    class="h-9 w-9 flex items-center justify-center rounded-lg bg-muted/60 text-muted-foreground opacity-50"
                >
                    <LuMaximize2 class="size-4" />
                </button>
                <MorphingDialogTrigger
                    v-else
                    class="h-9 w-9 flex items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted"
                >
                    <LuMaximize2 class="size-4" />
                </MorphingDialogTrigger>
                <MorphingDialogContainer>
                    <MorphingDialogContent class="relative">
                        <APIKeyDialogPanel />
                    </MorphingDialogContent>
                </MorphingDialogContainer>
            </MorphingDialog>
        </template>
    </APIKeyPanel>
</template>
