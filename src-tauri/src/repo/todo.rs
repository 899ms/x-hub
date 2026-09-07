use crate::models::Todo;
use crate::repo::now;
use rusqlite::{params, Connection, Result};

const COLS: &str =
    "id, title, done, priority, created_at, updated_at, completed_at, due_at, remind_at, remind_fired, parent_id, sort_order, version";

pub fn list(conn: &Connection) -> Result<Vec<Todo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM todos
         ORDER BY done ASC,
           CASE WHEN done = 1 THEN completed_at ELSE created_at END DESC,
           id DESC",
    ))?;
    let rows = stmt.query_map([], row_to_todo)?;
    rows.collect()
}

/// 创建待办；parent_id 非空时创建为该父条目下的子待办（父不存在报错）；
/// created_at 非空时保留原时间戳（删除撤销恢复用，避免恢复项排到最新位置）
pub fn create(
    conn: &Connection,
    title: &str,
    parent_id: Option<i64>,
    created_at: Option<&str>,
) -> Result<Todo> {
    if let Some(pid) = parent_id {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM todos WHERE id = ?1",
            params![pid],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "父待办不存在: {pid}"
            )));
        }
    }
    let ts = created_at
        .map(str::to_owned)
        .unwrap_or_else(now);
    conn.execute(
        "INSERT INTO todos (title, parent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        params![title, parent_id, ts],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Todo> {
    conn.query_row(
        &format!("SELECT {COLS} FROM todos WHERE id = ?1"),
        params![id],
        row_to_todo,
    )
}

pub fn update(conn: &Connection, id: i64, title: &str, priority: i64) -> Result<Todo> {
    conn.execute(
        "UPDATE todos SET title = ?1, priority = ?2, updated_at = ?3, version = version + 1 WHERE id = ?4",
        params![title, priority, now(), id],
    )?;
    get(conn, id)
}

/// 设置截止/提醒时刻（毫秒时间戳；NULL 表示清除）。
/// 每次排期都重置 remind_fired，用户改提醒时间后重新武装后台触发。
/// 同时清空手动排序位：截止日期决定分组归属，换组后原顺序语义失效，
/// 该条目回到新组默认位置（创建时间倒序）。
pub fn schedule(
    conn: &Connection,
    id: i64,
    due_at: Option<i64>,
    remind_at: Option<i64>,
) -> Result<Todo> {
    conn.execute(
        "UPDATE todos SET due_at = ?1, remind_at = ?2, remind_fired = 0, sort_order = NULL, updated_at = ?3, version = version + 1 WHERE id = ?4",
        params![due_at, remind_at, now(), id],
    )?;
    get(conn, id)
}

/// affected == 0 时区分「记录不存在」与「版本冲突」：若把 NOT_FOUND 误报成 CONFLICT，
/// 同步端会走「刷新重试」而记录根本不存在，永远失败。label 如「待办」。
fn not_found_or_conflict(conn: &Connection, id: i64, label: &str) -> String {
    let exists: Option<i64> = conn
        .query_row("SELECT 1 FROM todos WHERE id = ?1", params![id], |r| r.get(0))
        .ok();
    match exists {
        None => format!("NOT_FOUND: 待办 {id} 不存在"),
        Some(_) => format!("CONFLICT: {label} {id} 已被他人修改，请刷新后重试"),
    }
}

/// 乐观锁更新（局域网同步写回）：仅当记录的 version == expected_version 才更新并 +1，
/// 否则返回 CONFLICT 错误（带 prefix，供桥 handler / 扩展后端识别）。
/// 不校验（expected = None）时等价于普通更新，供主 UI 本地写回使用。
pub fn update_with_version(
    conn: &Connection,
    id: i64,
    title: &str,
    priority: i64,
    expected_version: Option<i64>,
) -> Result<Todo, String> {
    let affected = conn
        .execute(
            "UPDATE todos SET title = ?1, priority = ?2, updated_at = ?3, version = version + 1
             WHERE id = ?4 AND (?5 IS NULL OR version = ?5)",
            params![title, priority, now(), id, expected_version],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err(not_found_or_conflict(conn, id, "待办"));
    }
    get(conn, id).map_err(|e| e.to_string())
}

/// 乐观锁切换完成状态。expected_version 命中才翻转，否则 CONFLICT。
pub fn toggle_with_version(
    conn: &Connection,
    id: i64,
    expected_version: Option<i64>,
) -> Result<Todo, String> {
    let affected = conn
        .execute(
            "UPDATE todos SET
               done = CASE WHEN done = 1 THEN 0 ELSE 1 END,
               completed_at = CASE WHEN done = 1 THEN NULL ELSE strftime('%Y-%m-%d %H:%M:%f','now') END,
               updated_at = ?1, version = version + 1
             WHERE id = ?2 AND (?3 IS NULL OR version = ?3)",
            params![now(), id, expected_version],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err(not_found_or_conflict(conn, id, "待办"));
    }
    get(conn, id).map_err(|e| e.to_string())
}

/// 乐观锁删除（子待办级联）。expected_version 命中才删除，否则 CONFLICT；
/// 返回被级联删除的子待办 id（供同步端传播级联删除），与普通 delete 语义一致。
pub fn delete_with_version(
    conn: &Connection,
    id: i64,
    expected_version: Option<i64>,
) -> Result<Vec<i64>, String> {
    // 先记下级联子 id（删除后经 FK ON DELETE CASCADE 消失，无法再查）
    let kids = children_ids(conn, id).map_err(|e| e.to_string())?;
    let affected = conn
        .execute(
            "DELETE FROM todos WHERE id = ?1 AND (?2 IS NULL OR version = ?2)",
            params![id, expected_version],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err(not_found_or_conflict(conn, id, "待办"));
    }
    Ok(kids)
}

/// 乐观锁排期。expected_version 命中才更新，否则 CONFLICT。
pub fn schedule_with_version(
    conn: &Connection,
    id: i64,
    due_at: Option<i64>,
    remind_at: Option<i64>,
    expected_version: Option<i64>,
) -> Result<Todo, String> {
    let affected = conn
        .execute(
            "UPDATE todos SET due_at = ?1, remind_at = ?2, remind_fired = 0, sort_order = NULL,
               updated_at = ?3, version = version + 1
             WHERE id = ?4 AND (?5 IS NULL OR version = ?5)",
            params![due_at, remind_at, now(), id, expected_version],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err(not_found_or_conflict(conn, id, "待办"));
    }
    get(conn, id).map_err(|e| e.to_string())
}

pub fn toggle(conn: &Connection, id: i64) -> Result<Todo> {
    conn.execute(
        "UPDATE todos SET
           done = CASE WHEN done = 1 THEN 0 ELSE 1 END,
           completed_at = CASE WHEN done = 1 THEN NULL ELSE strftime('%Y-%m-%d %H:%M:%f','now') END,
           updated_at = ?1, version = version + 1
         WHERE id = ?2",
        params![now(), id],
    )?;
    get(conn, id)
}

/// 删除待办。子待办经外键 ON DELETE CASCADE 一并删除。
pub fn delete(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let kids = children_ids(conn, id)?;
    conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
    Ok(kids)
}

/// 直接子待办 id 列表（仅一层，无嵌套子待办）
pub fn children_ids(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM todos WHERE parent_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![id], |r| r.get(0))?;
    rows.collect()
}

/// 按传入顺序写入手动排序位（拖拽排序；ids[i] 的 sort_order = i+1）。
/// 只更新传入的条目——前端按分组计算顺序，后端不做分组解释。
pub fn reorder(conn: &Connection, ids: &[i64]) -> Result<()> {
    let ts = now();
    let tx = conn.unchecked_transaction()?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE todos SET sort_order = ?1, updated_at = ?2 WHERE id = ?3",
            params![(i + 1) as i64, ts, id],
        )?;
    }
    tx.commit()
}

pub fn search(conn: &Connection, keyword: &str) -> Result<Vec<Todo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM todos
         WHERE title LIKE '%' || ?1 || '%' ORDER BY done ASC, created_at DESC LIMIT 20",
    ))?;
    let rows = stmt.query_map(params![keyword], row_to_todo)?;
    rows.collect()
}

/// 到期待提醒的待办（未完成、未触发过、提醒时刻已到）。
/// 提醒只针对未完成项：完成后由 done 过滤，无需清 remind_at。
pub fn list_due_reminders(conn: &Connection, now_ms: i64) -> Result<Vec<Todo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM todos
         WHERE done = 0 AND remind_fired = 0 AND remind_at IS NOT NULL AND remind_at <= ?1
         ORDER BY remind_at ASC",
    ))?;
    let rows = stmt.query_map(params![now_ms], row_to_todo)?;
    rows.collect()
}

pub fn mark_remind_fired(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE todos SET remind_fired = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn row_to_todo(row: &rusqlite::Row) -> Result<Todo> {
    Ok(Todo {
        id: row.get(0)?,
        title: row.get(1)?,
        done: row.get(2)?,
        priority: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        completed_at: row.get(6)?,
        due_at: row.get(7)?,
        remind_at: row.get(8)?,
        remind_fired: row.get::<_, i64>(9)? != 0,
        parent_id: row.get(10)?,
        sort_order: row.get(11)?,
        version: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_in_memory;

    fn setup() -> Connection {
        init_in_memory().unwrap()
    }

    #[test]
    fn create_sets_defaults() {
        let conn = setup();
        let t = create(&conn, "写周报", None, None).unwrap();
        assert_eq!(t.title, "写周报");
        assert!(!t.done);
        assert_eq!(t.priority, 0);
        assert_eq!(t.completed_at, None);
        assert_eq!(t.due_at, None);
        assert_eq!(t.remind_at, None);
        assert!(!t.remind_fired);
        assert_eq!(t.parent_id, None);
        assert!(!t.created_at.is_empty());
    }

    #[test]
    fn create_preserves_provided_created_at() {
        // 删除撤销恢复：保留原 created_at，避免恢复项按新时间排到列表最新位置
        let conn = setup();
        let t = create(&conn, "恢复的待办", None, Some("2026-01-02 03:04:05.123456")).unwrap();
        assert_eq!(t.created_at, "2026-01-02 03:04:05.123456");
    }

    #[test]
    fn list_returns_all() {
        let conn = setup();
        create(&conn, "任务 A", None, None).unwrap();
        create(&conn, "任务 B", None, None).unwrap();
        let list = list(&conn).unwrap();
        assert_eq!(list.len(), 2);
        let titles: Vec<&str> = list.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"任务 A"));
        assert!(titles.contains(&"任务 B"));
    }

    #[test]
    fn toggle_marks_done_then_undone() {
        let conn = setup();
        let t = create(&conn, "洗衣服", None, None).unwrap();
        let done = toggle(&conn, t.id).unwrap();
        assert!(done.done);
        assert!(done.completed_at.is_some());
        let undone = toggle(&conn, t.id).unwrap();
        assert!(!undone.done);
        assert_eq!(undone.completed_at, None);
    }

    #[test]
    fn update_changes_title_and_priority() {
        let conn = setup();
        let t = create(&conn, "旧标题", None, None).unwrap();
        let updated = update(&conn, t.id, "新标题", 2).unwrap();
        assert_eq!(updated.title, "新标题");
        assert_eq!(updated.priority, 2);
    }

    #[test]
    fn delete_removes_todo() {
        let conn = setup();
        let t = create(&conn, "临时任务", None, None).unwrap();
        delete(&conn, t.id).unwrap();
        assert!(get(&conn, t.id).is_err());
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn search_matches_substring_and_percent() {
        let conn = setup();
        create(&conn, "买牛奶", None, None).unwrap();
        create(&conn, "进度 50%", None, None).unwrap();
        let found = search(&conn, "牛奶").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "买牛奶");
        // LIKE 中 % 是通配符，无需转义即可命中含 % 的标题
        let pct = search(&conn, "50%").unwrap();
        assert_eq!(pct.len(), 1);
        assert_eq!(pct[0].title, "进度 50%");
    }

    #[test]
    fn sub_todo_links_to_parent() {
        let conn = setup();
        let parent = create(&conn, "准备 PPT", None, None).unwrap();
        let sub = create(&conn, "完成初稿", Some(parent.id), None).unwrap();
        assert_eq!(sub.parent_id, Some(parent.id));
        // 重复父创建：父不存在时报错
        assert!(create(&conn, "孤儿", Some(99999), None).is_err());
    }

    #[test]
    fn delete_parent_cascades_children() {
        let conn = setup();
        let parent = create(&conn, "父待办", None, None).unwrap();
        let s1 = create(&conn, "子一", Some(parent.id), None).unwrap();
        let s2 = create(&conn, "子二", Some(parent.id), None).unwrap();
        let kids = delete(&conn, parent.id).unwrap();
        assert_eq!(kids, vec![s1.id, s2.id]);
        assert!(get(&conn, parent.id).is_err());
        assert!(get(&conn, s1.id).is_err());
        assert!(get(&conn, s2.id).is_err());
    }

    #[test]
    fn schedule_sets_and_clears_due_and_remind() {
        let conn = setup();
        let t = create(&conn, "交房租", None, None).unwrap();
        let due = 1_800_000_000_000;
        let remind = due - 30 * 60_000;
        let s = schedule(&conn, t.id, Some(due), Some(remind)).unwrap();
        assert_eq!(s.due_at, Some(due));
        assert_eq!(s.remind_at, Some(remind));
        assert!(!s.remind_fired);
        // 清除
        let cleared = schedule(&conn, t.id, None, None).unwrap();
        assert_eq!(cleared.due_at, None);
        assert_eq!(cleared.remind_at, None);
    }

    #[test]
    fn reorder_assigns_sort_order_in_given_sequence() {
        let conn = setup();
        let a = create(&conn, "任务 A", None, None).unwrap();
        let b = create(&conn, "任务 B", None, None).unwrap();
        let c = create(&conn, "任务 C", None, None).unwrap();
        // 新建条目未手动排序
        assert!(list(&conn).unwrap().iter().all(|t| t.sort_order.is_none()));
        reorder(&conn, &[c.id, a.id, b.id]).unwrap();
        let got = |id: i64| get(&conn, id).unwrap().sort_order;
        assert_eq!(got(c.id), Some(1));
        assert_eq!(got(a.id), Some(2));
        assert_eq!(got(b.id), Some(3));
    }

    #[test]
    fn schedule_clears_manual_sort_order() {
        // 截止日期决定分组归属，换组后原手动顺序失效，应回到默认排序
        let conn = setup();
        let t = create(&conn, "换组的任务", None, None).unwrap();
        reorder(&conn, &[t.id]).unwrap();
        assert_eq!(get(&conn, t.id).unwrap().sort_order, Some(1));
        let s = schedule(&conn, t.id, Some(1_800_000_000_000), None).unwrap();
        assert_eq!(s.sort_order, None);
    }

    #[test]
    fn due_reminders_skip_fired_and_done() {
        let conn = setup();
        let now = 1_800_000_000_000;
        let a = create(&conn, "到期未触发", None, None).unwrap();
        schedule(&conn, a.id, Some(now + 1), Some(now - 60_000)).unwrap();
        let b = create(&conn, "已触发", None, None).unwrap();
        schedule(&conn, b.id, None, Some(now - 60_000)).unwrap();
        mark_remind_fired(&conn, b.id).unwrap();
        let c = create(&conn, "已完成", None, None).unwrap();
        schedule(&conn, c.id, None, Some(now - 60_000)).unwrap();
        toggle(&conn, c.id).unwrap();
        let d = create(&conn, "未到点", None, None).unwrap();
        schedule(&conn, d.id, None, Some(now + 60_000)).unwrap();

        let due = list_due_reminders(&conn, now).unwrap();
        let titles: Vec<&str> = due.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["到期未触发"]);
    }

    #[test]
    fn update_with_version_optimistic_lock() {
        let conn = setup();
        let t = create(&conn, "原标题", None, None).unwrap();
        assert_eq!(t.version, 0);

        // 版本命中：更新并自增 version
        let updated = update_with_version(&conn, t.id, "新标题", 1, Some(0)).unwrap();
        assert_eq!(updated.title, "新标题");
        assert_eq!(updated.version, 1);

        // 版本不匹配：返回 CONFLICT，数据不被覆盖
        let err = update_with_version(&conn, t.id, "冲突写入", 2, Some(0)).unwrap_err();
        assert!(err.starts_with("CONFLICT"));
        let t2 = get(&conn, t.id).unwrap();
        assert_eq!(t2.title, "新标题");
        assert_eq!(t2.version, 1);

        // 不校验（None）等价普通更新
        let local = update_with_version(&conn, t.id, "本地直写", 0, None).unwrap();
        assert_eq!(local.title, "本地直写");
        assert_eq!(local.version, 2);
    }

    #[test]
    fn toggle_with_version_conflicts_on_stale_version() {
        let conn = setup();
        let t = create(&conn, "开关", None, None).unwrap();

        let done = toggle_with_version(&conn, t.id, Some(0)).unwrap();
        assert!(done.done);
        assert_eq!(done.version, 1);

        // 旧版本号切换：CONFLICT
        let err = toggle_with_version(&conn, t.id, Some(0)).unwrap_err();
        assert!(err.starts_with("CONFLICT"));
        assert!(get(&conn, t.id).unwrap().done);
    }

    #[test]
    fn delete_with_version_guards_stale_version() {
        let conn = setup();
        let t = create(&conn, "待删", None, None).unwrap();
        // 先改一次让 version=1
        update_with_version(&conn, t.id, "已改", 0, Some(0)).unwrap();

        // 用旧版本删除：CONFLICT，记录仍存在
        let err = delete_with_version(&conn, t.id, Some(0)).unwrap_err();
        assert!(err.starts_with("CONFLICT"));
        assert!(get(&conn, t.id).is_ok());

        // 命中版本删除成功
        delete_with_version(&conn, t.id, Some(1)).unwrap();
        assert!(get(&conn, t.id).is_err());
    }

    #[test]
    fn schedule_with_version_optimistic_lock() {
        let conn = setup();
        let t = create(&conn, "排期", None, None).unwrap();
        let due = 1_800_000_000_000;

        let s = schedule_with_version(&conn, t.id, Some(due), None, Some(0)).unwrap();
        assert_eq!(s.due_at, Some(due));
        assert_eq!(s.version, 1);

        // 旧版本排期：CONFLICT
        let err = schedule_with_version(&conn, t.id, None, None, Some(0)).unwrap_err();
        assert!(err.starts_with("CONFLICT"));
        assert_eq!(get(&conn, t.id).unwrap().due_at, Some(due));
    }

    #[test]
    fn local_ui_writes_bump_version() {
        // 主 UI 本地写（旧函数）也必须推进版本链：否则同步端持过期 expectedVersion
        // 仍能「命中」写回，本机 UI 刚做的修改被静默覆盖
        let conn = setup();
        let t = create(&conn, "原始", None, None).unwrap();
        assert_eq!(t.version, 0);

        let u = update(&conn, t.id, "改标题", 2).unwrap();
        assert_eq!(u.version, 1);

        let g = toggle(&conn, t.id).unwrap();
        assert_eq!(g.version, 2);

        let s = schedule(&conn, t.id, Some(123_456), None).unwrap();
        assert_eq!(s.version, 3);
    }

    #[test]
    fn with_version_reports_not_found_for_missing_id() {
        // 记录不存在必须报 NOT_FOUND 而非 CONFLICT（同步端不应反复重试一条已删记录）
        let conn = setup();
        let err = update_with_version(&conn, 9999, "x", 0, Some(0)).unwrap_err();
        assert!(err.starts_with("NOT_FOUND"), "{err}");
        let err = toggle_with_version(&conn, 9999, None).unwrap_err();
        assert!(err.starts_with("NOT_FOUND"), "{err}");
        let err = schedule_with_version(&conn, 9999, None, None, None).unwrap_err();
        assert!(err.starts_with("NOT_FOUND"), "{err}");
        let err = delete_with_version(&conn, 9999, None).unwrap_err();
        assert!(err.starts_with("NOT_FOUND"), "{err}");

        // 对照：记录存在但版本不匹配仍报 CONFLICT
        let t = create(&conn, "存在", None, None).unwrap();
        let err = update_with_version(&conn, t.id, "x", 0, Some(5)).unwrap_err();
        assert!(err.starts_with("CONFLICT"), "{err}");
    }

    #[test]
    fn delete_with_version_returns_cascade_children() {
        let conn = setup();
        let parent = create(&conn, "父", None, None).unwrap();
        let kid = create(&conn, "子", Some(parent.id), None).unwrap();

        let kids = delete_with_version(&conn, parent.id, None).unwrap();
        assert_eq!(kids, vec![kid.id]);
        assert!(get(&conn, parent.id).is_err());
    }
}
