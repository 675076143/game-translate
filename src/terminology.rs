pub fn translate_battle(text: &str) -> Option<String> {
    if let Some(name) = text
        .strip_prefix("Go! ")
        .and_then(|text| text.strip_suffix('!'))
    {
        return Some(format!("上吧！{}！", pokemon(name)));
    }
    if let Some(body) = text.strip_suffix(" fainted!") {
        let (prefix, name) = battle_subject(body);
        return Some(format!("{prefix}{}倒下了！", pokemon(name)));
    }
    if let Some(body) = text.strip_suffix('!')
        && let Some((name, move_name)) = body.split_once(" used ")
    {
        let (prefix, name) = battle_subject(name);
        return Some(format!(
            "{prefix}{}使用了{}！",
            pokemon(name),
            battle_move(move_name)
        ));
    }
    if let Some(body) = text.strip_suffix(" Exp. Points!")
        && let Some((name, amount)) = body.split_once(" got ")
    {
        return Some(format!("{}获得了{amount}点经验值！", pokemon(name)));
    }
    if let Some(name) = text
        .strip_prefix("A wild ")
        .and_then(|text| text.strip_suffix(" appeared!"))
    {
        return Some(format!("野生的{}出现了！", pokemon(name)));
    }
    if let Some(name) = text.strip_suffix(" was caught!") {
        return Some(format!("抓到了{}！", pokemon(name.trim_start_matches("Gotcha! "))));
    }
    if let Some(item) = text
        .strip_prefix("You found an ")
        .and_then(|text| text.strip_suffix('!'))
    {
        return Some(format!("你找到了{}！", item_name(item)));
    }
    None
}

fn battle_subject(text: &str) -> (&'static str, &str) {
    if let Some(name) = text.strip_prefix("The wild ") {
        ("野生的", name)
    } else if let Some(name) = text.strip_prefix("The opposing ") {
        ("对手的", name)
    } else {
        ("", text)
    }
}

fn pokemon(name: &str) -> &str {
    match name {
        "Charmander" => "小火龙",
        "Hoothoot" => "咕咕",
        "Rattata" => "小拉达",
        "Pidgey" => "波波",
        "Tangela" => "蔓藤怪",
        "Pichu" => "皮丘",
        "Weedle" => "独角虫",
        "Caterpie" => "绿毛虫",
        _ => name,
    }
}

fn item_name(name: &str) -> &str {
    match name {
        "Antidote" => "解毒药",
        "Potion" => "伤药",
        "Poké Ball" | "Poke Ball" => "精灵球",
        _ => name,
    }
}

fn battle_move(name: &str) -> &str {
    match name {
        "Ember" => "火花",
        "Foresight" => "识破",
        "Tackle" => "撞击",
        "Sand Attack" => "泼沙",
        "Constrict" => "缠绕",
        "Scratch" => "抓",
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::translate_battle;

    #[test]
    fn translates_known_battle_templates() {
        assert_eq!(
            translate_battle("The wild Pidgey used Tackle!").unwrap(),
            "野生的波波使用了撞击！"
        );
        assert_eq!(
            translate_battle("The wild Rattata fainted!").unwrap(),
            "野生的小拉达倒下了！"
        );
        assert_eq!(translate_battle("Go! Charmander!").unwrap(), "上吧！小火龙！");
        assert_eq!(
            translate_battle("Charmander got 10 Exp. Points!").unwrap(),
            "小火龙获得了10点经验值！"
        );
        assert_eq!(
            translate_battle("A wild Pichu appeared!").unwrap(),
            "野生的皮丘出现了！"
        );
        assert_eq!(
            translate_battle("You found an Antidote!").unwrap(),
            "你找到了解毒药！"
        );
    }
}
