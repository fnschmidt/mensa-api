use anyhow::{Context, Result, anyhow};
use chrono::{Datelike, NaiveDate};
use lazy_static::lazy_static;
use scraper::{Element, ElementRef, Html, Selector};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::constants::CANTEEN_MAP_INV;
use crate::db_operations::{add_canteen_id_db, get_jsonmeals_from_db, save_meal_to_db};
use crate::types::{
    CanteenMealDiff, CanteenMealsDay, HasChanges, MealGroup, MealVariation, SingleMeal,
};

pub async fn _run_benchmark() -> Result<()> {
    println!("downloading htmls");
    let today = chrono::Local::now();

    let mut strings: Vec<String> = Vec::new();
    for i in 0..7 {
        let day: chrono::DateTime<chrono::FixedOffset> = (today + chrono::Duration::days(i)).into();

        if ![chrono::Weekday::Sat, chrono::Weekday::Sun].contains(&day.weekday()) {
            strings.push(reqwest_get_html_text(&build_date_string(day.date_naive())).await?);
        }
    }

    println!("got {} htmls", strings.len());
    let now = Instant::now();
    let its = 100;

    // ST
    for _ in 0..its {
        for string in &strings {
            extract_data_from_html(string).await?;
        }
    }

    println!("{} in {:.2?}", its * strings.len(), now.elapsed());

    Ok(())
}

pub async fn parse_and_save_meals(day: NaiveDate) -> Result<Vec<CanteenMealDiff>> {
    let mut today_changed_canteen_diffs = vec![];

    let date_string = build_date_string(day);

    // getting data from server
    let downloaded_html = reqwest_get_html_text(&date_string).await?;

    let all_canteen_singleday = extract_data_from_html(&downloaded_html).await?;
    // serialize downloaded meals
    for canteen_meals_singleday in all_canteen_singleday {
        let downloaded_json_text =
            serde_json::to_string(&canteen_meals_singleday.meal_groups).unwrap();
        let db_json_text =
            get_jsonmeals_from_db(&date_string, canteen_meals_singleday.canteen_id).await?;

        // if downloaded meals are different from cached meals, update cache
        // if db_json_text.is_none() || downloaded_json_text != db_json_text.unwrap() {
        if db_json_text.is_none() || downloaded_json_text != *db_json_text.as_ref().unwrap() {
            log::info!(
                "updating cache: Canteen={} Date={}",
                canteen_meals_singleday.canteen_id,
                date_string
            );
            save_meal_to_db(
                &date_string,
                canteen_meals_singleday.canteen_id,
                &downloaded_json_text,
            )
            .await?;

            if day.weekday() == chrono::Local::now().weekday() {
                let old_meals = db_json_text
                    .map(|text| serde_json::from_str::<Vec<MealGroup>>(&text).unwrap())
                    .map(|old_mealgroups| CanteenMealsDay {
                        canteen_id: canteen_meals_singleday.canteen_id,
                        meal_groups: old_mealgroups,
                    });

                let diff = diff_canteen_meals(old_meals.as_ref(), &canteen_meals_singleday);
                if diff.has_changes() {
                    today_changed_canteen_diffs.push(diff);
                } else if old_meals.is_some() {
                    log::warn!("DB != downloaded data, but diffing found nothing!");
                }
            }
        }
    }

    Ok(today_changed_canteen_diffs)
}

pub fn diff_canteen_meals(
    old_canteenmeals: Option<&CanteenMealsDay>,
    new_canteenmeals: &CanteenMealsDay,
) -> CanteenMealDiff {
    let mut new_meals: Vec<MealGroup> = vec![];
    let mut modified_meals: Vec<MealGroup> = vec![];
    let mut modified_meals_ignoring_allergens: Vec<MealGroup> = vec![];
    let mut removed_meals: Vec<MealGroup> = vec![];

    if let Some(old_canteenmeals) = old_canteenmeals {
        assert_eq!(old_canteenmeals.canteen_id, new_canteenmeals.canteen_id);
        for new_mealgroup in &new_canteenmeals.meal_groups {
            let equiv_old_mealgroups = old_canteenmeals
                .meal_groups
                .iter()
                .find(|old_group| old_group.meal_type == new_mealgroup.meal_type);
            if equiv_old_mealgroups.is_none() {
                // new category → all submeals are new
                new_meals.push(new_mealgroup.clone());
            } else {
                // find new submeals
                let new_or_changed_submeals =
                    new_mealgroup.sub_meals.iter().filter(|new_submeal| {
                        equiv_old_mealgroups
                            .unwrap()
                            .sub_meals
                            .iter()
                            .all(|old_submeal| (old_submeal != *new_submeal))
                    });

                let new_or_changed_ignoring_allergens =
                    new_mealgroup.sub_meals.iter().filter(|new_submeal| {
                        equiv_old_mealgroups
                            .unwrap()
                            .sub_meals
                            .iter()
                            .all(|old_submeal| {
                                old_submeal.name != *new_submeal.name
                                    || old_submeal.additional_ingredients
                                        != *new_submeal.additional_ingredients
                                    || old_submeal.variations != new_submeal.variations
                                    || old_submeal.price != *new_submeal.price
                            })
                    });

                let (changed_submeals, new_submeals): (Vec<_>, Vec<_>) = new_or_changed_submeals
                    .partition(|meal| {
                        equiv_old_mealgroups
                            .unwrap()
                            .sub_meals
                            .iter()
                            .any(|old_submeal| old_submeal.name == meal.name)
                    });

                let changed_submeals_ignoring_allergens = new_or_changed_ignoring_allergens
                    .filter(|meal| {
                        equiv_old_mealgroups
                            .unwrap()
                            .sub_meals
                            .iter()
                            .any(|old_submeal| old_submeal.name == meal.name)
                    })
                    .collect::<Vec<_>>();

                if !changed_submeals.is_empty() {
                    modified_meals.push(MealGroup {
                        meal_type: new_mealgroup.meal_type.clone(),
                        sub_meals: changed_submeals.into_iter().cloned().collect(),
                    });
                }

                if !changed_submeals_ignoring_allergens.is_empty() {
                    modified_meals_ignoring_allergens.push(MealGroup {
                        meal_type: new_mealgroup.meal_type.clone(),
                        sub_meals: changed_submeals_ignoring_allergens
                            .into_iter()
                            .cloned()
                            .collect(),
                    });
                }

                if !new_submeals.is_empty() {
                    new_meals.push(MealGroup {
                        meal_type: new_mealgroup.meal_type.clone(),
                        sub_meals: new_submeals.into_iter().cloned().collect(),
                    });
                }

                // find removed submeals if the category already exists in old data
                let removed_submeals: Vec<_> = equiv_old_mealgroups
                    .unwrap()
                    .sub_meals
                    .iter()
                    .filter(|old_submeal| {
                        new_mealgroup
                            .sub_meals
                            .iter()
                            .all(|new_submeal| new_submeal.name != old_submeal.name)
                    })
                    .cloned()
                    .collect();

                if !removed_submeals.is_empty() {
                    removed_meals.push(MealGroup {
                        meal_type: new_mealgroup.meal_type.clone(),
                        sub_meals: removed_submeals,
                    });
                }
            }
        }

        // find removed categories
        for old_meal_group in &old_canteenmeals.meal_groups {
            if !new_canteenmeals
                .meal_groups
                .iter()
                .any(|new_group| new_group.meal_type == old_meal_group.meal_type)
            {
                removed_meals.push(old_meal_group.clone());
            }
        }
    }

    CanteenMealDiff {
        canteen_id: new_canteenmeals.canteen_id,
        new_meals: if new_meals.is_empty() {
            None
        } else {
            Some(new_meals)
        },
        modified_meals: if modified_meals.is_empty() {
            None
        } else {
            Some(modified_meals)
        },
        modified_meals_ignoring_allergens: if modified_meals_ignoring_allergens.is_empty() {
            None
        } else {
            Some(modified_meals_ignoring_allergens)
        },
        removed_meals: if removed_meals.is_empty() {
            None
        } else {
            Some(removed_meals)
        },
    }
}

pub fn build_date_string(requested_date: NaiveDate) -> String {
    let (year, month, day) = (
        requested_date.year(),
        requested_date.month(),
        requested_date.day(),
    );

    format!("{:04}-{:02}-{:02}", year, month, day)
}

async fn reqwest_get_html_text(date: &str) -> Result<String> {
    let now = Instant::now();
    let url_base =
        "https://www.studentenwerk-leipzig.de/mensen-cafeterien/speiseplan?date=".to_string();
    let txt = reqwest::get(url_base + date).await?.text().await?;

    log::debug!("reqwest_get_html_text: {:.2?}", now.elapsed());
    Ok(txt)
}

async fn extract_data_from_html(html_text: &str) -> Result<Vec<CanteenMealsDay>> {
    let mut all_data_for_day = vec![];

    let now = Instant::now();

    let document = Html::parse_fragment(html_text);

    lazy_static! {
        static ref DATE_BUTTON_GROUPSEL: Selector =
            Selector::parse(r#"button.date-button.is--active"#).unwrap();
        static ref TITLE_SEL: Selector = Selector::parse("h3").unwrap();
    };

    document
        .select(&DATE_BUTTON_GROUPSEL)
        .next()
        .context("Recv. StuWe site is invalid (has no date)")?;

    let title_elements = document.select(&TITLE_SEL);

    for canteen_name_el in title_elements {
        let canteen_name = canteen_name_el.inner_html();
        let meals = extract_mealgroup_from_htmlcontainer(
            canteen_name_el
                .next_sibling_element()
                .context("h3 without meal container")?,
        )?;

        let canteen_map_inv_r = CANTEEN_MAP_INV.read().unwrap();
        let canteen_id = canteen_map_inv_r.get(&canteen_name).copied().unwrap_or({
            // drop here, otherwise drop would occur after the write lock (≙ dead lock)
            drop(canteen_map_inv_r);
            let extr_id = extract_canteenid(&document, &canteen_name)?;

            if CANTEEN_MAP_INV
                .write()
                .unwrap()
                .insert(canteen_name.clone(), extr_id)
                // race conditions between writers and readers can cause two readers to think
                // the value needs to be added (only writes lock exclusively)
                .is_none()
            {
                log::info!("Adding new canteen to db: {}", canteen_name);
                add_canteen_id_db(extr_id, &canteen_name)?;
            };

            extr_id
        });

        all_data_for_day.push(CanteenMealsDay {
            canteen_id,
            meal_groups: meals,
        });
    }

    log::info!("HTML → Data: {:.2?}", now.elapsed());
    Ok(all_data_for_day)
}

fn extract_mealgroup_from_htmlcontainer(meal_container: ElementRef<'_>) -> Result<Vec<MealGroup>> {
    let mut v_meal_groups: Vec<MealGroup> = Vec::new();

    lazy_static! {
        static ref MEAL_SEL: Selector = Selector::parse(r#"div.type--meal"#).unwrap();
        static ref MEAL_TYPE_SEL: Selector = Selector::parse(r#"div.meal-tags>.tag"#).unwrap();
        static ref TITLE_SEL: Selector = Selector::parse(r#"h4"#).unwrap();
        static ref ADDITIONAL_INGREDIENTS_SEL: Selector =
            Selector::parse(r#"div.meal-components"#).unwrap();
        static ref PRICE_SEL: Selector = Selector::parse(r#"div.meal-prices>span"#).unwrap();
        static ref ALLERGENS_SEL: Selector = Selector::parse(r#"div.meal-allergens>p"#).unwrap();
        static ref VARIATIONS_SEL: Selector = Selector::parse(r#"div.meal-subitems"#).unwrap();
        static ref H5_SELECTOR: Selector = Selector::parse("h5").unwrap();
        static ref P_SELECTOR: Selector = Selector::parse("p").unwrap();
    };

    // quick && dirty
    for meal_element in meal_container.select(&MEAL_SEL) {
        let meal_type = meal_element
            .select(&MEAL_TYPE_SEL)
            .next()
            .context("meal category element not found")?
            .inner_html();

        let title = meal_element
            .select(&TITLE_SEL)
            .next()
            .context("meal title element not found")?
            .inner_html()
            .replace("&nbsp;", " ")
            .replace("&amp;", "&");

        let additional_ingredients =
            if let Some(item) = meal_element.select(&ADDITIONAL_INGREDIENTS_SEL).next() {
                let text = item.inner_html();
                // for whatever reason there might be, sometimes this element exists without any content
                if !text.is_empty() {
                    let mut add_ingr_dedup: Vec<String> = vec![];
                    let inner_html = item.inner_html();
                    let iter = inner_html.split('·').map(|slice| slice.trim().to_string());
                    for ingr in iter {
                        let clean = ingr.replace("&nbsp;", " ").replace("&amp; ", "");
                        let clean = clean
                            .strip_prefix("/")
                            .map(|s| s.to_string())
                            .unwrap_or(clean);
                        if !add_ingr_dedup.contains(&clean) {
                            add_ingr_dedup.push(clean);
                        }
                    }

                    add_ingr_dedup
                } else {
                    // in that case, return empty vec (otherwise it would be a vec with one empty string in it)
                    vec![]
                }
                // Sosumi
            } else {
                vec![]
            };

        let mut price = String::new();
        meal_element.select(&PRICE_SEL).for_each(|price_element| {
            price += &price_element
                .inner_html()
                .replace("&nbsp;", " ")
                .replace("&amp;", "&");
        });
        price = price.trim().to_string();

        let allergens = meal_element
            .select(&ALLERGENS_SEL)
            .next()
            .map(|el| el.inner_html());

        let variations = meal_element.select(&VARIATIONS_SEL).next().map(|el| {
            let mut variations_vec: Vec<MealVariation> = vec![];

            for variation in el.child_elements() {
                let name = variation
                    .select(&H5_SELECTOR)
                    .next()
                    .unwrap()
                    .text()
                    .next()
                    .unwrap()
                    .trim()
                    .to_string();

                let allergens_and_add = variation
                    .select(&P_SELECTOR)
                    .next()
                    .map(|el| el.text().last().unwrap().replace(": ", "").to_string());

                let variation = MealVariation {
                    name,
                    allergens_and_add,
                };

                // deduplicate here as well
                if !variations_vec.contains(&variation) {
                    variations_vec.push(variation);
                }
            }

            variations_vec
        });

        // oh my
        // oh my
        match v_meal_groups
            .iter_mut()
            .find(|meal_group| meal_group.meal_type == meal_type)
        {
            None => {
                // doesn't exist yet, create new meal group of new meal type
                v_meal_groups.push(MealGroup {
                    meal_type,
                    sub_meals: vec![SingleMeal {
                        name: title,
                        price,
                        additional_ingredients,
                        allergens,
                        variations,
                    }],
                });
            }
            Some(meal_group) => {
                // meal group of this type already exists, add meal to it

                let add_meal = SingleMeal {
                    name: title,
                    price,
                    additional_ingredients,
                    allergens,
                    variations,
                };

                if !meal_group.sub_meals.contains(&add_meal) {
                    meal_group.sub_meals.push(add_meal);
                }
            }
        }
    }

    Ok(v_meal_groups)
}

fn extract_canteenid(document: &Html, canteen_title: &str) -> Result<u32> {
    lazy_static! {
        static ref CANTEEN_LIST_SEL: Selector = Selector::parse("#locations>li").unwrap();
    };

    let canteen_li = document
        .select(&CANTEEN_LIST_SEL)
        .find(|li| li.first_element_child().unwrap().inner_html() == canteen_title);
    if let Some(canteen_li) = canteen_li {
        if let Some(canteen_id) = canteen_li.value().attr("data-location") {
            if let Ok(canteen_id) = canteen_id.parse::<u32>() {
                return Ok(canteen_id);
            }
        }
    }

    Err(anyhow!("Failed to extract canteen id"))
}

pub fn invert_map<K, V>(map: &BTreeMap<K, V>) -> BTreeMap<V, K>
where
    K: Clone + Ord,
    V: Clone + Ord,
{
    map.iter().map(|(k, v)| (v.clone(), k.clone())).collect()
}
