<script setup lang="ts">
import type { RelayLogOverview } from '@/api/log';
import { MorphingDialog } from '@/components/ui/morphing-dialog';
import LogCardBody from './LogCardBody.vue';

export interface LogCardProps {
    log: RelayLogOverview;
}

defineProps<LogCardProps>();
</script>

<template>
    <MorphingDialog>
        <LogCardBody :log="log" />
    </MorphingDialog>
</template>
