<script setup lang="ts">
import { LuX } from 'vue-icons-plus/lu';
import { useMorphingDialog } from '@/components/ui/morphing-dialog';
import APIKeyPanel from './APIKeyPanel.vue';

const { setIsOpen } = useMorphingDialog();
</script>

<template>
    <APIKeyPanel
        id-prefix="apikey-dialog"
        container-class-name="rounded-3xl border border-border bg-card p-6 space-y-5 relative w-screen max-w-full md:max-w-xl"
        list-class-name="space-y-2 h-[calc(100vh-10rem)] overflow-y-auto"
    >
        <template #headerExtra>
            <button
                type="button"
                @click="() => setIsOpen(false)"
                class="h-9 w-9 flex items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted"
                title="Close"
            >
                <LuX class="size-4" />
            </button>
        </template>
    </APIKeyPanel>
</template>
