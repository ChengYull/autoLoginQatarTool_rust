//! 密码池：7 个密码的轮换、强度校验、随机生成。
//!
//! 业务背景（1.0.5 新增）：
//!   * 服务端密码 **7 天过期**，过期后无法登录，必须重新设密码；
//!   * 服务端规定 **同一个密码在最近一个月内不能重复使用**，
//!     撞上时返回「该密码在最近一个月使用过。请尝试新密码!」。
//!
//! 所以这里维护一个 7 个密码的环，每 7 天往前走一格：
//! 一个密码两次被使用的最小间隔是 7 × 7 = 49 天 > 31 天，
//! 只要用户填的 7 个密码互不相同，纯自动轮换**永远不会**撞上「最近一个月」。
//!
//! 会撞车的只有两种情况，都在 [`Pool::candidates`] 里处理掉：
//!   1. 用户中途用「自定义密码」把服务端密码改成了列表里的某一个；
//!   2. 用户手工调整过列表，打乱了顺序。
//! 这两种情况下，靠 `used[]` 里记的时间戳就能跳过还在窗口内的密码。

use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

/// 密码池容量。7 个 × 7 天 = 49 天一轮回。
pub const POOL_SIZE: usize = 7;

/// 重置周期（秒）
pub const PERIOD_SECS: i64 = 7 * 24 * 3600;

/// 提前量：不能卡在整 7 天那一刻才动手。
///
/// 服务端按自己的时钟判过期，本地时钟只要慢几分钟、或者网络抖一下，
/// 「到期了才重置」就会变成「已经过期了还没重置」—— 而密码一过期，
/// 客户端连登录都进不去。留 6 小时足够覆盖时区/时钟误差。
pub const LEAD_SECS: i64 = 6 * 3600;

/// 服务端「最近一个月」的重置窗口。按 31 天算，宁可保守。
pub const REUSE_WINDOW_SECS: i64 = 31 * 24 * 3600;

/// 长度下限。服务端只说了「不超过 16 位」，下限取 8 位，比常见策略略宽松，
/// 避免把用户已经在用的旧密码判成不合规。
pub const MIN_LEN: usize = 8;
/// 长度上限（服务端明确要求）
pub const MAX_LEN: usize = 16;

/// 生成器产出的长度。取中间值：够强，也不会长到记不住。
pub const GEN_LEN: usize = 12;

/// 算作「特殊符号」的字符集合。只收键盘上常见、各家系统都不会拒的那批。
pub const SPECIALS: &str = "._-@#$%^&*!";

// 生成用的字符集：剔除 0/O、1/l/I 这类肉眼分不清的字符，
// 因为用户很可能要照着屏幕往别的设备上抄。
const UPPER: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijkmnopqrstuvwxyz";
const DIGIT: &str = "23456789";

/// 当前 Unix 时间（秒）
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 强度校验
// ---------------------------------------------------------------------------

/// 校验一个密码是否符合服务端规范。
///
/// 返回 `Err` 时给的是**能直接显示给用户**的原因，界面拿它做输入框爆红 + 提示文案。
pub fn validate(pwd: &str) -> Result<(), String> {
    let len = pwd.chars().count();
    if len == 0 {
        return Err("密码不能为空".into());
    }
    if len > MAX_LEN {
        return Err(format!("不能超过 {} 位（当前 {} 位）", MAX_LEN, len));
    }
    if len < MIN_LEN {
        return Err(format!("至少 {} 位（当前 {} 位）", MIN_LEN, len));
    }
    // 空白字符在 HTTP 表单 / 复制粘贴里最容易出问题，直接禁掉
    if pwd.chars().any(|c| c.is_whitespace()) {
        return Err("不能包含空格".into());
    }
    // 非 ASCII 一律拒绝：中文、全角符号、emoji 都可能被服务端判成不合规
    if let Some(c) = pwd.chars().find(|c| !c.is_ascii()) {
        return Err(format!("包含不支持的字符「{}」", c));
    }
    if !pwd.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("缺少大写字母".into());
    }
    if !pwd.chars().any(|c| c.is_ascii_lowercase()) {
        return Err("缺少小写字母".into());
    }
    if !pwd.chars().any(|c| c.is_ascii_digit()) {
        return Err("缺少数字".into());
    }
    if !pwd.chars().any(|c| SPECIALS.contains(c)) {
        return Err(format!("缺少特殊符号（{}）", SPECIALS));
    }
    Ok(())
}

/// 校验整个密码池：7 项都要合规、互不相同。
///
/// 互不相同是硬要求 —— 重复的密码会让「49 天一轮回」提前撞上服务端的
/// 「最近一个月使用过」，自动重置就卡死了。
pub fn validate_pool(list: &[String]) -> Result<(), (usize, String)> {
    if list.len() != POOL_SIZE {
        return Err((0, format!("需要 {} 个密码", POOL_SIZE)));
    }
    for (i, pwd) in list.iter().enumerate() {
        if let Err(msg) = validate(pwd) {
            return Err((i, msg));
        }
    }
    for i in 0..list.len() {
        for j in (i + 1)..list.len() {
            if list[i] == list[j] {
                return Err((j, format!("与第 {} 个密码重复", i + 1)));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 随机生成
// ---------------------------------------------------------------------------

/// 取 `n` 个密码学随机字节（BCryptGenRandom，走系统 CSPRNG）
fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    // 失败时退化成时间戳派生的伪随机：绝不能因为拿不到随机数就让「生成」按钮失灵
    let ok = unsafe {
        BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG).is_ok()
    };
    if !ok {
        let seed = now_unix() as u64 ^ (std::process::id() as u64) << 17;
        let mut x = seed | 1;
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
    buf
}

/// 从字符集里随机取一个字符
fn pick(set: &str, rnd: &mut impl FnMut() -> u8) -> char {
    let chars: Vec<char> = set.chars().collect();
    chars[rnd() as usize % chars.len()]
}

/// 生成一个符合规范的随机密码。
///
/// 保证四类字符各至少一个（先各放一个打底，再补齐长度、最后洗牌），
/// 而不是「随机生成完再检查」—— 后者在极端参数下可能反复失败。
pub fn generate() -> String {
    let bytes = random_bytes(GEN_LEN * 4 + 16);
    let mut cursor = 0usize;
    let mut next = || {
        let b = bytes[cursor % bytes.len()];
        cursor += 1;
        b
    };

    let mut out: Vec<char> = Vec::with_capacity(GEN_LEN);
    out.push(pick(UPPER, &mut next));
    out.push(pick(LOWER, &mut next));
    out.push(pick(DIGIT, &mut next));
    out.push(pick(SPECIALS, &mut next));

    let all = format!("{}{}{}{}", UPPER, LOWER, DIGIT, SPECIALS);
    while out.len() < GEN_LEN {
        out.push(pick(&all, &mut next));
    }

    // Fisher-Yates 洗牌，否则前四位永远是「大写小写数字符号」的固定套路
    for i in (1..out.len()).rev() {
        let j = next() as usize % (i + 1);
        out.swap(i, j);
    }

    out.into_iter().collect()
}

// ---------------------------------------------------------------------------
// 密码池
// ---------------------------------------------------------------------------

/// 密码池状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    /// 7 个密码，互不相同
    pub list: [String; POOL_SIZE],
    /// 当前生效的密码下标
    pub index: usize,
    /// 上次成功重置的时间（Unix 秒）。0 = 从未重置 / 未启用
    pub last_reset: i64,
    /// 每个密码上次生效的时间（Unix 秒），0 = 没用过。
    /// 用来绕开服务端「最近一个月使用过」的限制。
    pub used: [i64; POOL_SIZE],
    /// 自动重置总开关
    pub auto: bool,
}

impl Default for Pool {
    fn default() -> Self {
        Pool {
            list: std::array::from_fn(|_| String::new()),
            index: 0,
            last_reset: 0,
            used: [0; POOL_SIZE],
            auto: false,
        }
    }
}

impl Pool {
    /// 7 个密码是否都已填好 —— 决定自动重置能不能跑
    pub fn is_configured(&self) -> bool {
        self.list.iter().all(|p| !p.is_empty())
    }

    /// 当前生效的密码
    #[allow(dead_code)]
    pub fn current(&self) -> &str {
        &self.list[self.index.min(POOL_SIZE - 1)]
    }

    /// 该动手重置的时间点（已含提前量）
    pub fn due_at(&self) -> i64 {
        if self.last_reset <= 0 {
            return 0;
        }
        self.last_reset + PERIOD_SECS - LEAD_SECS
    }

    /// 是否已到该重置的时候
    pub fn is_due(&self, now: i64) -> bool {
        let due = self.due_at();
        due > 0 && now >= due
    }

    /// 距离该重置还有几天（向上取整，已过期返回 0）。界面上显示「下次重置：X 天后」。
    pub fn days_until_due(&self, now: i64) -> i64 {
        let due = self.due_at();
        if due <= 0 {
            return 0;
        }
        let left = due - now;
        if left <= 0 {
            0
        } else {
            (left + 86_399) / 86_400
        }
    }

    /// 候选密码下标，按轮换顺序排列。
    ///
    /// 从当前下标的下一个开始绕一圈，跳过 `REUSE_WINDOW_SECS` 内用过的
    /// （服务端会拒绝）。正常情况下第一个就是答案；只有撞上「最近一个月
    /// 使用过」时才需要往后取。
    pub fn candidates(&self, now: i64) -> Vec<usize> {
        let mut out = Vec::with_capacity(POOL_SIZE);
        for step in 1..=POOL_SIZE {
            let i = (self.index + step) % POOL_SIZE;
            if self.list[i].is_empty() {
                continue;
            }
            if self.used[i] == 0 || now.saturating_sub(self.used[i]) >= REUSE_WINDOW_SECS {
                out.push(i);
            }
        }
        out
    }

    /// 把某个下标标成「当前生效」：更新下标、使用时间、重置时间。
    ///
    /// 只有服务端确认设置成功后才该调用 —— 本地时间和服务端密码必须严格同步，
    /// 否则自动登录会拿着一个服务端根本不认的密码去填。
    pub fn mark_active(&mut self, i: usize, now: i64) {
        if i >= POOL_SIZE {
            return;
        }
        self.index = i;
        self.used[i] = now;
        self.last_reset = now;
    }

    /// 只记时间、不改下标：用于「首次保存时立即应用第 1 个」且第 1 个
    /// 恰好就是当前密码的情况 —— 服务端那边已经是这个密码了，
    /// 再去 setpassword 反而会被「最近一个月使用过」拒掉。
    pub fn mark_current_in_use(&mut self, now: i64) {
        self.used[self.index] = now;
        self.last_reset = now;
    }

    /// 打乱后的 7 个随机密码
    pub fn generate_all() -> [String; POOL_SIZE] {
        let mut out: [String; POOL_SIZE] = std::array::from_fn(|_| String::new());
        for i in 0..POOL_SIZE {
            // 理论上不会撞，撞了就重来一次，代价可以忽略
            for _ in 0..32 {
                let candidate = generate();
                let taken = out[..i].iter().any(|p| p == &candidate);
                if !taken {
                    out[i] = candidate;
                    break;
                }
            }
        }
        out
    }

    // -- 序列化 ------------------------------------------------------------
    //
    // 自定义的极简行式格式（不引入 serde：这个项目要保证单 exe、编译快）：
    //
    //   v1
    //   <index>
    //   <last_reset>
    //   <auto 0/1>
    //   <used0>,<used1>,...,<used6>
    //   <pwd0>
    //   ...
    //   <pwd6>
    //
    // 密码本身禁止空白/控制字符（`validate` 保证），所以按行切分是安全的。

    pub fn to_text(&self) -> String {
        let used: Vec<String> = self.used.iter().map(|v| v.to_string()).collect();
        let mut s = String::new();
        s.push_str("v1\n");
        s.push_str(&format!("{}\n", self.index));
        s.push_str(&format!("{}\n", self.last_reset));
        s.push_str(&format!("{}\n", if self.auto { 1 } else { 0 }));
        s.push_str(&format!("{}\n", used.join(",")));
        for pwd in self.list.iter() {
            s.push_str(pwd);
            s.push('\n');
        }
        s
    }

    /// 解析失败返回 `None` —— 调用方当成「没配置过」处理，
    /// 绝不能拿半截数据去跑自动重置。
    pub fn from_text(text: &str) -> Option<Pool> {
        let mut lines = text.lines();
        if lines.next()? != "v1" {
            return None;
        }
        let index: usize = lines.next()?.trim().parse().ok()?;
        let last_reset: i64 = lines.next()?.trim().parse().ok()?;
        let auto = lines.next()?.trim() == "1";
        let used_line = lines.next()?;
        let mut used = [0i64; POOL_SIZE];
        for (i, part) in used_line.split(',').enumerate() {
            if i >= POOL_SIZE {
                return None;
            }
            used[i] = part.trim().parse().ok()?;
        }
        let mut list: [String; POOL_SIZE] = std::array::from_fn(|_| String::new());
        for slot in list.iter_mut() {
            *slot = lines.next()?.to_string();
        }
        if index >= POOL_SIZE {
            return None;
        }
        Some(Pool { list, index, last_reset, used, auto })
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Pool {
        let mut p = Pool::default();
        for (i, slot) in p.list.iter_mut().enumerate() {
            *slot = format!("TestPass{}a.", i);
        }
        p
    }

    #[test]
    fn validate_accepts_normal_password() {
        assert!(validate("ChengYu111.").is_ok());
        assert!(validate("Abcdef1!").is_ok());
        assert!(validate("Abcdef1!Abcdef1!").is_ok()); // 正好 16
    }

    #[test]
    fn validate_rejects_bad_ones() {
        assert!(validate("").is_err());
        assert!(validate("Abc1.").is_err(), "太短");
        assert!(validate("Abcdefg1.12345678").is_err(), "17 位超长");
        assert!(validate("abcdefg1.").is_err(), "缺大写");
        assert!(validate("ABCDEFG1.").is_err(), "缺小写");
        assert!(validate("Abcdefgh.").is_err(), "缺数字");
        assert!(validate("Abcdefg12").is_err(), "缺特殊符号");
        assert!(validate("Abc def1.").is_err(), "含空格");
        assert!(validate("Abcdef1中").is_err(), "含中文");
    }

    #[test]
    fn generated_password_is_valid_and_unique() {
        let all = Pool::generate_all();
        let mut seen = std::collections::HashSet::new();
        for pwd in all.iter() {
            assert!(validate(pwd).is_ok(), "生成器产出不合规: {pwd}");
            assert!(seen.insert(pwd.clone()), "生成器产出了重复密码");
        }
    }

    #[test]
    fn pool_validation_catches_duplicates() {
        let mut list: Vec<String> = (0..POOL_SIZE).map(|i| format!("TestPass{i}a.")).collect();
        assert!(validate_pool(&list).is_ok());
        list[3] = list[0].clone();
        let err = validate_pool(&list).unwrap_err();
        assert_eq!(err.0, 3);
        assert!(err.1.contains("重复"));
    }

    #[test]
    fn rotation_skips_recently_used() {
        let mut p = sample();
        let now = 1_000_000_000;
        p.index = 0;
        p.last_reset = now;
        p.used[0] = now;
        // 正常情况下第一个候选就是下一个
        assert_eq!(p.candidates(now)[0], 1);

        // 把 1、2 标成 10 天前用过（仍在 31 天窗口内），应被跳过
        p.used[1] = now - 10 * 86400;
        p.used[2] = now - 10 * 86400;
        let cand = p.candidates(now);
        assert_eq!(cand[0], 3, "应跳过一个月内用过的密码");
        assert!(!cand.contains(&1) && !cand.contains(&2));
    }

    #[test]
    fn rotation_wraps_and_never_reuses_within_window() {
        let mut p = sample();
        let mut now = 1_700_000_000;
        p.mark_active(0, now);
        // 走完整整一圈，每次都应该拿到下一个，且永远不会撞上 31 天窗口
        for expect in 1..=POOL_SIZE {
            now += PERIOD_SECS;
            let cand = p.candidates(now);
            assert!(!cand.is_empty(), "候选为空");
            assert_eq!(cand[0], expect % POOL_SIZE);
            p.mark_active(cand[0], now);
        }
        // 走完一圈后 index 已回到 0，下一次（第 56 天）应该用 1；
        // 0 的上次使用是第 49 天，间隔 49 天 > 31 天，届时才允许再用
        now += PERIOD_SECS;
        assert_eq!(p.candidates(now)[0], 1);
    }

    #[test]
    fn due_time_has_lead_margin() {
        let mut p = sample();
        let now = 1_700_000_000;
        p.mark_active(0, now);
        assert!(!p.is_due(now + PERIOD_SECS - LEAD_SECS - 1));
        assert!(p.is_due(now + PERIOD_SECS - LEAD_SECS));
        // 6.75 天 → 显示 1 天，而不是 0
        assert_eq!(p.days_until_due(now + PERIOD_SECS - LEAD_SECS - 1), 1);
        assert_eq!(p.days_until_due(now), 7);
    }

    #[test]
    fn unconfigured_pool_is_never_due() {
        let p = Pool::default();
        assert!(!p.is_configured());
        assert!(!p.is_due(now_unix()));
        assert_eq!(p.due_at(), 0);
    }

    #[test]
    fn text_roundtrip() {
        let mut p = sample();
        p.index = 3;
        p.last_reset = 1_700_000_123;
        p.used[3] = 1_700_000_123;
        p.auto = true;
        let back = Pool::from_text(&p.to_text()).expect("解析失败");
        assert_eq!(p, back);
    }

    #[test]
    fn bad_text_is_rejected() {
        assert!(Pool::from_text("").is_none());
        assert!(Pool::from_text("v2\n0\n0\n0\n0,0,0,0,0,0,0\na\n").is_none());
        assert!(Pool::from_text("v1\n99\n0\n0\n0,0,0,0,0,0,0\na\n").is_none());
    }
}
