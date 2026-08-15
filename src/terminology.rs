pub fn translate_battle(text: &str) -> Option<String> {
    if let Some(name) = text
        .strip_prefix("Go! ")
        .and_then(|text| text.strip_suffix('!'))
    {
        return Some(format!("上吧！{}ï¼", pokemon(name)));
    }
    if let Some(body) = text.strip_suffix(" fainted!") {
        let name = body.strip_prefix("The wild ")?;
        return Some(format!("野生的{}倒下了！", pokemon(name)));
    }
    if let Some(body) = text.strip_suffix('!')
        && let Some((name, move_name)) = body.split_once(" used ")
    {
        let name = name.strip_prefix("The wild ").unwrap_or(name);
        return Some(format!(
            "{}使用了{}！",
            pokemon(name),
            battle_move(move_name)
        ));
    }
    None
}

fn pokemon(name: &str) -> &str {
    match name {
        "Charmander" => "小火龙",
        "Hoothoot" => "咕咕",
        "Rattata" => "小拉达",
        "Pidgey" => "波波",
        "Tangela" => "蔓藤怪",
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
            "波波使用了撞击！"
        );
        assert_eq!(
            translate_battle("The wild Rattata fainted!").unwrap(),
            "野生的小拉达倒下了！"
        );
    }
}
