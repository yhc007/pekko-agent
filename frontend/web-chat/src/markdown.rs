/// 경량 마크다운 → HTML 변환기
/// 지원: 테이블, **볼드**, `코드`, ```코드블록```, 줄바꿈
pub fn markdown_to_html(input: &str) -> String {
    let mut result = String::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // ── 코드 블록 (```) ──
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim();
            result.push_str("<pre><code");
            if !lang.is_empty() {
                result.push_str(&format!(" class=\"lang-{}\"", escape_html(lang)));
            }
            result.push('>');
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                result.push_str(&escape_html(lines[i]));
                result.push('\n');
                i += 1;
            }
            result.push_str("</code></pre>");
            i += 1;
            continue;
        }

        // ── 마크다운 테이블 ──
        if line.contains('|') && is_table_row(line) {
            // 테이블 시작 감지
            let mut table_rows: Vec<&str> = vec![line];
            i += 1;

            // 구분선 행 (|---|---| 등) 탐지
            let has_separator = i < lines.len() && is_separator_row(lines[i]);
            if has_separator {
                i += 1; // 구분선 건너뛰기
            }

            // 나머지 테이블 행 수집
            while i < lines.len() && lines[i].contains('|') && is_table_row(lines[i]) {
                table_rows.push(lines[i]);
                i += 1;
            }

            // 테이블 렌더링
            result.push_str("<div class=\"table-wrapper\"><table>");

            for (ri, row) in table_rows.iter().enumerate() {
                let cells = parse_table_cells(row);
                let tag = if ri == 0 && (has_separator || table_rows.len() > 1) {
                    "th"
                } else {
                    "td"
                };

                if ri == 0 {
                    result.push_str("<thead><tr>");
                } else if ri == 1 {
                    // 첫 번째 tbody 행: tbody 열기
                    result.push_str("<tbody><tr>");
                } else {
                    result.push_str("<tr>");
                }

                for cell in &cells {
                    result.push_str(&format!("<{tag}>{}</{tag}>", inline_format(cell.trim())));
                }

                if ri == 0 {
                    result.push_str("</tr></thead>");
                } else {
                    result.push_str("</tr>");
                }
            }

            if table_rows.len() > 1 {
                result.push_str("</tbody>");
            }
            result.push_str("</table></div>");
            continue;
        }

        // ── 일반 텍스트 행 ──
        if line.trim().is_empty() {
            result.push_str("<br/>");
        } else {
            result.push_str("<p>");
            result.push_str(&inline_format(line));
            result.push_str("</p>");
        }
        i += 1;
    }

    result
}

/// 테이블 행인지 판단
fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    // | 로 시작하거나 내부에 | 가 있으면 테이블 행으로 간주
    trimmed.starts_with('|') || trimmed.matches('|').count() >= 2
}

/// 구분선 행인지 판단 (|---|---| 형태)
fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    // |, -, :, 공백만으로 구성되면 구분선
    trimmed.chars().all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

/// 테이블 행에서 셀 추출
fn parse_table_cells(row: &str) -> Vec<String> {
    let trimmed = row.trim();
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') {
        &trimmed[1..trimmed.len() - 1]
    } else if trimmed.starts_with('|') {
        &trimmed[1..]
    } else if trimmed.ends_with('|') {
        &trimmed[..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner.split('|').map(|s| s.to_string()).collect()
}

/// 인라인 포맷팅: **볼드**, `코드`
fn inline_format(text: &str) -> String {
    let escaped = escape_html(text);

    // **볼드** 처리
    let mut result = String::new();
    let parts: Vec<&str> = escaped.split("**").collect();
    if parts.len() > 1 {
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                result.push_str(&format!("<strong>{part}</strong>"));
            } else {
                result.push_str(part);
            }
        }
    } else {
        result = escaped;
    }

    // `인라인 코드`
    let mut final_result = String::new();
    let code_parts: Vec<&str> = result.split('`').collect();
    if code_parts.len() > 1 {
        for (i, part) in code_parts.iter().enumerate() {
            if i % 2 == 1 {
                final_result.push_str(&format!("<code>{part}</code>"));
            } else {
                final_result.push_str(part);
            }
        }
    } else {
        final_result = result;
    }

    final_result
}

/// HTML 이스케이프
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
