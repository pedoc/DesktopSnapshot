<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ElMessage } from "element-plus";
import {
  Camera,
  Delete,
  FolderOpened,
  Picture,
  Refresh,
  RefreshRight,
  Timer,
  Tools,
} from "@element-plus/icons-vue";

type SnapshotSummary = {
  id: string;
  title: string;
  createdUnixSeconds: number;
  itemCount: number;
};

type BackupResult = {
  id: string;
  itemCount: number;
  backedUpCount: number;
  failedItems: string[];
  screenshotCount: number;
};

type RestoreResult = {
  id: string;
  restoredCount: number;
  skippedCount: number;
  failedCount: number;
  layoutRestored: boolean;
};

const snapshots = ref<SnapshotSummary[]>([]);
const selectedSnapshot = ref<SnapshotSummary | null>(null);
const previewUrls = ref<string[]>([]);
const fullImageUrls = ref<string[]>([]);
const previewVisible = ref(false);
const previewIndex = ref(0);
const deleteDialogVisible = ref(false);
const renameDialogVisible = ref(false);
const renameTitle = ref("");
const busy = ref(false);
const fencesInstalled = ref(false);
const status = ref("正在读取快照...");

const hasSelection = computed(() => Boolean(selectedSnapshot.value));

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString();
}

function showError(error: unknown, prefix: string) {
  status.value = `${prefix}：${String(error)}`;
  ElMessage.error(status.value);
}

async function loadSnapshots() {
  snapshots.value = await invoke<SnapshotSummary[]>("list_snapshots");
  selectedSnapshot.value = null;
  previewUrls.value = [];
  fullImageUrls.value = [];
  status.value = snapshots.value.length ? "请选择一个快照。" : "暂无快照，请先创建快照。";
}

async function selectSnapshot(row: SnapshotSummary | null) {
  selectedSnapshot.value = row;
  previewUrls.value = [];
  fullImageUrls.value = [];
  if (!row) return;
  try {
    const [thumbnails, fullImages] = await Promise.all([
      invoke<string[]>("get_snapshot_images", { snapshotId: row.id, thumbnail: true }),
      invoke<string[]>("get_snapshot_images", { snapshotId: row.id, thumbnail: false }),
    ]);
    previewUrls.value = thumbnails;
    fullImageUrls.value = fullImages;
    status.value = `已选择快照：${row.title}`;
  } catch {
    status.value = `已选择快照：${row.title}（没有截图）`;
  }
}

async function runBusy<T>(action: () => Promise<T>, message: (result: T) => string) {
  busy.value = true;
  try {
    const result = await action();
    status.value = message(result);
    ElMessage.success(status.value);
    await loadSnapshots();
  } catch (error) {
    showError(error, "操作失败");
  } finally {
    busy.value = false;
  }
}

async function createSnapshot() {
  const appWindow = getCurrentWindow();
  await runBusy(async () => {
    await appWindow.hide();
    try {
      return await invoke<BackupResult>("create_snapshot");
    } finally {
      await appWindow.show();
    }
  }, (result) => `已创建快照 #${result.id}，备份 ${result.itemCount} 个项目，截图 ${result.screenshotCount} 张。`);
}

async function restoreSelected() {
  if (!selectedSnapshot.value) {
    ElMessage.warning("请先选择一个快照。");
    return;
  }
  const snapshotId = selectedSnapshot.value.id;
  busy.value = true;
  try {
    const result = await invoke<RestoreResult>("restore_snapshot", { snapshotId });
    status.value = result.elevationRequested
      ? `已请求管理员权限恢复快照 #${result.id}，请在 UAC 对话框中确认。`
      : `已恢复 #${result.id}：恢复 ${result.restoredCount} 个，跳过 ${result.skippedCount} 个，失败 ${result.failedCount} 个。`;
    if (result.failedItems.length) {
      const preview = result.failedItems.slice(0, 5).join("；");
      ElMessage.warning(`部分项目恢复失败：${preview}`);
    } else {
      ElMessage.success(status.value);
    }
    await loadSnapshots();
  } catch (error) {
    showError(error, "恢复失败");
  } finally {
    busy.value = false;
  }
}
async function openBackupDirectory() {
  try {
    const path = await invoke<string>("open_backup_directory");
    status.value = `已打开备份目录：${path}`;
    ElMessage.success(status.value);
  } catch (error) {
    showError(error, "打开备份目录失败");
  }
}

async function registerBackupTask() {
  try {
    await invoke("register_auto_backup", { intervalMinutes: 15 });
    status.value = "已注册每 15 分钟自动备份任务。";
    ElMessage.success(status.value);
  } catch (error) {
    showError(error, "任务注册失败");
  }
}

function requestRename() {
  if (!selectedSnapshot.value) {
    ElMessage.warning("请先选择一个快照。");
    return;
  }
  renameTitle.value = selectedSnapshot.value.title;
  renameDialogVisible.value = true;
}

async function confirmRename() {
  if (!selectedSnapshot.value || !renameTitle.value.trim()) {
    ElMessage.warning("请输入快照名称。");
    return;
  }
  const snapshotId = selectedSnapshot.value.id;
  const title = renameTitle.value.trim();
  renameDialogVisible.value = false;
  await runBusy(
    () => invoke<SnapshotSummary>("rename_snapshot", { snapshotId, title }),
    (result) => `已将快照重命名为“${result.title}”。`,
  );
}
function requestDelete() {
  if (!selectedSnapshot.value) {
    ElMessage.warning("请先选择一个快照。");
    return;
  }
  deleteDialogVisible.value = true;
}

async function confirmDelete() {
  if (!selectedSnapshot.value) return;
  const snapshotId = selectedSnapshot.value.id;
  deleteDialogVisible.value = false;
  await runBusy(
    () => invoke<SnapshotSummary>("delete_snapshot", { snapshotId }),
    (result) => `已删除快照 #${result.id}。`,
  );
}

onMounted(async () => {
  try {
    await loadSnapshots();
  } catch (error) {
    showError(error, "读取快照失败");
  }
});
</script>

<template>
  <main class="app-shell">
    <header class="hero-card">
      <div class="brand-mark"><Picture /></div>
      <div class="hero-copy">
        <h1>DesktopSnapshot</h1>
        <p>桌面项目、文件内容与图标布局快照工具</p>
      </div>
      <el-tag class="hero-tag" type="info" effect="plain">本地备份</el-tag>
    </header>

    <el-alert class="status-alert" :title="status" :type="busy ? 'warning' : 'info'" :closable="false" show-icon />

    <section class="toolbar-card">
      <el-button type="primary" :icon="Camera" :loading="busy" @click="createSnapshot">
        创建快照
      </el-button>
      <el-button :icon="RefreshRight" :disabled="busy || !hasSelection" @click="restoreSelected">
        恢复选中快照
      </el-button>
      <el-button :icon="Refresh" :loading="busy" @click="loadSnapshots">刷新</el-button>
      <el-button :icon="Edit" :disabled="busy || !hasSelection" @click="requestRename">重命名</el-button>
      <el-button :icon="FolderOpened" :disabled="busy" @click="openBackupDirectory">
        打开目录
      </el-button>
      <el-button :icon="Timer" :disabled="busy" @click="registerBackupTask">
        注册自动备份
      </el-button>
      <el-button type="danger" plain :icon="Delete" :disabled="busy || !hasSelection" @click="requestDelete">
        删除选中快照
      </el-button>
    </section>

    <el-alert
      v-if="fencesInstalled"
      class="fences-alert"
      type="warning"
      title="检测到 Stardock Fences"
      description="文件和快捷方式仍可备份、恢复；Fences 分组和图标位置由 Fences 自己管理，当前程序无法保证位置恢复。"
      :closable="false"
      show-icon
    />

    <section class="content-grid">
      <el-card class="table-card" shadow="never">
        <template #header>
          <div class="card-title">
            <span>备份快照</span>
            <el-tag size="small">{{ snapshots.length }} 个</el-tag>
          </div>
        </template>
        <el-table
          :data="snapshots"
          row-key="id"
          highlight-current-row
          height="430"
          empty-text="暂无快照"
          @current-change="selectSnapshot"
        >
          <el-table-column prop="title" label="快照" min-width="220" show-overflow-tooltip />
          <el-table-column label="创建时间" min-width="180">
            <template #default="{ row }">{{ formatTime(row.createdUnixSeconds) }}</template>
          </el-table-column>
          <el-table-column prop="itemCount" label="项目数" width="90" />
        </el-table>
      </el-card>

      <el-card class="preview-card" shadow="never">
        <template #header>
          <div class="card-title">
            <span>快照预览</span>
            <el-tag v-if="selectedSnapshot" type="success" size="small">已选择</el-tag>
          </div>
        </template>
        <div v-if="previewUrls.length" class="preview-wrap">
          <div class="preview-gallery">
            <button
              v-for="(preview, index) in previewUrls"
              :key="index"
              class="preview-tile"
              type="button"
            >
              <el-image
                class="preview-image"
                :src="preview"
                fit="contain"
                alt="桌面快照预览"
                :preview-src-list="fullImageUrls"
                :initial-index="index"
                :zoom-rate="1.2"
                :min-scale="0.2"
                :max-scale="7"
                preview-teleported
                show-progress
              />
              <span>显示器 {{ index + 1 }} · 点击缩放</span>
            </button>
          </div>
        </div>
        <el-empty v-else description="选择有截图的快照查看预览" :image-size="90" />
      </el-card>
    </section>


    <el-dialog v-model="renameDialogVisible" title="修改快照名称" width="430px">
      <el-input v-model="renameTitle" maxlength="120" show-word-limit @keyup.enter="confirmRename" />
      <template #footer>
        <el-button @click="renameDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="busy" @click="confirmRename">保存</el-button>
      </template>
    </el-dialog>
    <el-dialog v-model="deleteDialogVisible" title="确认删除快照" width="430px">
      <p class="dialog-text">
        即将删除“{{ selectedSnapshot?.title }}”，包含 {{ selectedSnapshot?.itemCount }} 个桌面项目。
      </p>
      <p class="dialog-warning">此操作会删除本地备份内容，且不可撤销。</p>
      <template #footer>
        <el-button @click="deleteDialogVisible = false">取消</el-button>
        <el-button type="danger" :loading="busy" @click="confirmDelete">确认删除</el-button>
      </template>
    </el-dialog>


  </main>
</template>
