use std::hash::BuildHasherDefault;

type BumpHashMap<'a, K, V> = hashbrown::HashMap<K, V, BuildHasherDefault<rustc_hash::FxHasher>, &'a bumpalo::Bump>;

use super::WriteItem;
use super::collections::*;
use super::infinite_reevaluation_protection::InfiniteReevaluationProtector;
use super::print_items::*;
use super::thread_state;
use super::thread_state::BumpAllocator;
use super::writer::*;

#[derive(Clone, Copy)]
enum ResolutionState<'a> {
  Condition(u32, Option<Option<bool>>),
  LineNumberAnchor(u32, Option<u32>),
  LineNumber(u32, Option<u32>),
  ColumnNumber(u32, Option<u32>),
  IsStartOfLine(u32, Option<bool>),
  IndentLevel(u32, Option<u8>),
  LineStartColumnNumber(u32, Option<u32>),
  LineStartIndentLevel(u32, Option<u8>),
  StoredConditionSavePoint(u32, Option<(&'a Condition, &'a SavePoint<'a>)>),
}

pub struct SavePoint<'a> {
  #[cfg(debug_assertions)]
  /// Name for debugging purposes.
  pub name: &'static str,
  pub new_line_group_depth: u16,
  pub force_no_newlines_depth: u8,
  pub writer_state: WriterState<'a>,
  pub possible_new_line_save_point: Option<&'a SavePoint<'a>>,
  pub node: Option<PrintItemPath>,
  pub look_ahead_condition_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_line_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_column_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_is_start_of_line_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_indent_level_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_line_start_column_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub look_ahead_line_start_indent_level_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  pub next_node_stack: NodeStack<'a>,
  pub resolutions_len: usize,
}

#[cfg(feature = "tracing")]
pub struct PrintTracingResult<'a> {
  pub traces: Vec<Trace>,
  pub writer_nodes: Vec<&'a GraphNode<'a, WriteItem<'a>>>,
}

/// Options for printing.
pub struct PrinterOptions {
  /// The width the printer will attempt to keep the line under.
  pub max_width: u32,
  /// The number of columns to count when indenting or using a tab.
  pub indent_width: u8,
  #[cfg(feature = "tracing")]
  pub enable_tracing: bool,
}

pub struct Printer<'a> {
  bump: &'a BumpAllocator,
  possible_new_line_save_point: Option<&'a SavePoint<'a>>,
  new_line_group_depth: u16,
  force_no_newlines_depth: u8,
  current_node: Option<PrintItemPath>,
  writer: Writer<'a>,
  // Use a hash map here because only some conditions are stored (not all).
  resolved_conditions: BumpHashMap<'a, u32, Option<bool>>,
  // Use these "VecU32Map" for resolved infos because it has much faster
  // lookups than a hash map and generally infos seem to be resolved
  // about 90% of the time, so the extra memory usage is probably not
  // a big deal.
  resolved_line_number_anchors: VecU32U32Map,
  resolved_line_numbers: VecU32U32Map,
  resolved_column_numbers: VecU32U32Map,
  resolved_is_start_of_lines: VecU32BoolMap,
  resolved_indent_levels: VecU32U8Map,
  resolved_line_start_column_numbers: VecU32U32Map,
  resolved_line_start_indent_levels: VecU32U8Map,
  look_ahead_condition_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_line_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_column_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_is_start_of_line_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_indent_level_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_line_start_column_number_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  look_ahead_line_start_indent_level_save_points: BumpHashMap<'a, u32, &'a SavePoint<'a>>,
  infinite_reevaluation_protector: InfiniteReevaluationProtector,
  next_node_stack: NodeStack<'a>,
  stored_condition_save_points: BumpHashMap<'a, u32, (&'a Condition, &'a SavePoint<'a>)>,
  resolution_log: Vec<ResolutionState<'a>>,
  max_width: u32,
  skip_moving_next: bool,
  resolving_save_point: Option<&'a SavePoint<'a>>,
  #[cfg(feature = "tracing")]
  traces: Option<Vec<Trace>>,
  #[cfg(feature = "tracing")]
  start_time: std::time::Instant,
}

impl<'a> Printer<'a> {
  pub fn new(bump: &'a BumpAllocator, start_node: Option<PrintItemPath>, options: PrinterOptions) -> Printer<'a> {
    Printer {
      bump,
      possible_new_line_save_point: None,
      new_line_group_depth: 0,
      force_no_newlines_depth: 0,
      current_node: start_node,
      writer: Writer::new(
        bump,
        WriterOptions {
          indent_width: options.indent_width,
          #[cfg(feature = "tracing")]
          enable_tracing: options.enable_tracing,
        },
      ),
      resolved_conditions: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      resolved_line_number_anchors: VecU32U32Map::with_capacity(thread_state::next_line_number_anchor_id()),
      resolved_line_numbers: VecU32U32Map::with_capacity(thread_state::next_line_number_id()),
      resolved_column_numbers: VecU32U32Map::with_capacity(thread_state::next_column_number_id()),
      resolved_is_start_of_lines: VecU32BoolMap::with_capacity(thread_state::next_is_start_of_line_id()),
      resolved_indent_levels: VecU32U8Map::with_capacity(thread_state::next_indent_level_id()),
      resolved_line_start_column_numbers: VecU32U32Map::with_capacity(thread_state::next_line_start_column_number_id()),
      resolved_line_start_indent_levels: VecU32U8Map::with_capacity(thread_state::next_line_start_indent_level_id()),
      look_ahead_condition_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_line_number_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_column_number_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_is_start_of_line_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_indent_level_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_line_start_column_number_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      look_ahead_line_start_indent_level_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      infinite_reevaluation_protector: InfiniteReevaluationProtector::with_capacity(thread_state::next_condition_reevaluation_id()),
      stored_condition_save_points: BumpHashMap::with_hasher_in(Default::default(), bump.inner()),
      next_node_stack: NodeStack::default(),
      resolution_log: Vec::new(),
      max_width: options.max_width,
      skip_moving_next: false,
      resolving_save_point: None,
      #[cfg(feature = "tracing")]
      traces: if options.enable_tracing { Some(Vec::new()) } else { None },
      #[cfg(feature = "tracing")]
      start_time: std::time::Instant::now(),
    }
  }

  /// Turns the print items into a collection of writer items according to the options.
  pub fn print(mut self) -> Option<impl Iterator<Item = WriteItem<'a>>> {
    self.inner_print();
    self.writer.items()
  }

  /// Turns the print items into a collection of writer items according to the options along with traces.
  #[cfg(feature = "tracing")]
  pub fn print_for_tracing(mut self) -> PrintTracingResult<'a> {
    self.inner_print();

    PrintTracingResult {
      traces: self.traces.expect("Should have set enable_tracing to true when creating the printer."),
      writer_nodes: self.writer.nodes(),
    }
  }

  fn inner_print(&mut self) {
    while let Some(current_node) = &self.current_node {
      let current_node = unsafe { &*current_node.get_node() }; // ok because values won't be mutated while printing
      self.handle_print_node(current_node);

      #[cfg(feature = "tracing")]
      self.create_trace(current_node);

      // println!("{}", self.writer.to_string_for_debugging());

      if self.skip_moving_next {
        self.skip_moving_next = false;
      } else if let Some(current_node) = self.current_node {
        self.current_node = current_node.get_next();
      }

      while self.current_node.is_none() && !self.next_node_stack.is_empty() {
        self.current_node = self.next_node_stack.pop();
      }
    }

    #[cfg(debug_assertions)]
    self.verify_no_look_ahead_save_points();
    #[cfg(debug_assertions)]
    self.ensure_counts_zero();
  }

  #[cfg(feature = "tracing")]
  fn create_trace(&mut self, current_node: &PrintNode) {
    if let Some(traces) = self.traces.as_mut() {
      traces.push(Trace {
        nanos: (std::time::Instant::now() - self.start_time).as_nanos(),
        print_node_id: current_node.print_node_id,
        writer_node_id: self.writer.current_node_id(),
      });
    }
  }

  #[inline]
  pub fn get_writer_info(&self) -> WriterInfo {
    self.writer.writer_info()
  }

  pub fn resolved_line_number(&mut self, line_number: LineNumber) -> Option<u32> {
    let resolved_number = self.resolved_line_numbers.get(line_number.unique_id());
    if resolved_number.is_none() && !self.look_ahead_line_number_save_points.contains_key(&line_number.unique_id()) {
      let save_point = self.get_save_point_for_restoring_condition(line_number.name());
      self.look_ahead_line_number_save_points.insert(line_number.unique_id(), save_point);
    }

    resolved_number
  }

  pub fn resolved_column_number(&mut self, column_number: ColumnNumber) -> Option<u32> {
    let resolved_number = self.resolved_column_numbers.get(column_number.unique_id());
    if resolved_number.is_none() && !self.look_ahead_column_number_save_points.contains_key(&column_number.unique_id()) {
      let save_point = self.get_save_point_for_restoring_condition(column_number.name());
      self.look_ahead_column_number_save_points.insert(column_number.unique_id(), save_point);
    }

    resolved_number
  }

  pub fn resolved_is_start_of_line(&mut self, is_start_of_line: IsStartOfLine) -> Option<bool> {
    let resolved_is_start_of_line = self.resolved_is_start_of_lines.get(is_start_of_line.unique_id());
    if resolved_is_start_of_line.is_none() && !self.look_ahead_is_start_of_line_save_points.contains_key(&is_start_of_line.unique_id()) {
      let save_point = self.get_save_point_for_restoring_condition(is_start_of_line.name());
      self.look_ahead_is_start_of_line_save_points.insert(is_start_of_line.unique_id(), save_point);
    }

    resolved_is_start_of_line
  }

  pub fn resolved_indent_level(&mut self, indent_level: IndentLevel) -> Option<u8> {
    let resolved_indent_level = self.resolved_indent_levels.get(indent_level.unique_id());
    if resolved_indent_level.is_none() && !self.look_ahead_indent_level_save_points.contains_key(&indent_level.unique_id()) {
      let save_point = self.get_save_point_for_restoring_condition(indent_level.name());
      self.look_ahead_indent_level_save_points.insert(indent_level.unique_id(), save_point);
    }

    resolved_indent_level
  }

  pub fn resolved_line_start_column_number(&mut self, line_start_column_number: LineStartColumnNumber) -> Option<u32> {
    let resolved_line_start_column_number = self.resolved_line_start_column_numbers.get(line_start_column_number.unique_id());
    if resolved_line_start_column_number.is_none()
      && !self
        .look_ahead_line_start_column_number_save_points
        .contains_key(&line_start_column_number.unique_id())
    {
      let save_point = self.get_save_point_for_restoring_condition(line_start_column_number.name());
      self
        .look_ahead_line_start_column_number_save_points
        .insert(line_start_column_number.unique_id(), save_point);
    }

    resolved_line_start_column_number
  }

  pub fn resolved_line_start_indent_level(&mut self, line_start_indent_level: LineStartIndentLevel) -> Option<u8> {
    let resolved_line_start_indent_level = self.resolved_line_start_indent_levels.get(line_start_indent_level.unique_id());
    if resolved_line_start_indent_level.is_none()
      && !self
        .look_ahead_line_start_indent_level_save_points
        .contains_key(&line_start_indent_level.unique_id())
    {
      let save_point = self.get_save_point_for_restoring_condition(line_start_indent_level.name());
      self
        .look_ahead_line_start_indent_level_save_points
        .insert(line_start_indent_level.unique_id(), save_point);
    }

    resolved_line_start_indent_level
  }

  pub fn clear_info(&mut self, info: Info) {
    match info {
      Info::LineNumber(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::LineNumber(id, self.resolved_line_numbers.get(id)));
        self.resolved_line_numbers.remove(id);
      }
      Info::ColumnNumber(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::ColumnNumber(id, self.resolved_column_numbers.get(id)));
        self.resolved_column_numbers.remove(id);
      }
      Info::IsStartOfLine(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::IsStartOfLine(id, self.resolved_is_start_of_lines.get(id)));
        self.resolved_is_start_of_lines.remove(id);
      }
      Info::IndentLevel(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::IndentLevel(id, self.resolved_indent_levels.get(id)));
        self.resolved_indent_levels.remove(id);
      }
      Info::LineStartColumnNumber(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::LineStartColumnNumber(id, self.resolved_line_start_column_numbers.get(id)));
        self.resolved_line_start_column_numbers.remove(id);
      }
      Info::LineStartIndentLevel(info) => {
        let id = info.unique_id();
        self.resolution_log.push(ResolutionState::LineStartIndentLevel(id, self.resolved_line_start_indent_levels.get(id)));
        self.resolved_line_start_indent_levels.remove(id);
      }
    }
  }

  pub fn resolved_condition(&mut self, condition_reference: &ConditionReference) -> Option<bool> {
    if !self.resolved_conditions.contains_key(&condition_reference.id) && !self.look_ahead_condition_save_points.contains_key(&condition_reference.id) {
      let save_point = self.get_save_point_for_restoring_condition(condition_reference.name());
      self.look_ahead_condition_save_points.insert(condition_reference.id, save_point);
    }

    let result = self.resolved_conditions.get(&condition_reference.id)?;
    result.map(|x| x.to_owned())
  }

  pub fn is_forcing_no_newlines(&self) -> bool {
    self.force_no_newlines_depth > 0
  }

  #[inline]
  fn handle_print_node(&mut self, print_node: &PrintNode) {
    match &print_node.item {
      PrintItem::String(text) => self.handle_string(text),
      PrintItem::Condition(condition) => self.handle_condition(condition, &print_node.next),
      PrintItem::Signal(signal) => self.handle_signal(signal),
      PrintItem::RcPath(rc_path) => self.handle_rc_path(rc_path, &print_node.next),
      PrintItem::Anchor(anchor) => self.handle_anchor(anchor),
      PrintItem::Info(info) => self.handle_targeted_info(info),
      PrintItem::ConditionReevaluation(reevaluation) => self.handle_condition_reevaluation(reevaluation),
    }
  }

  fn write_new_line(&mut self) {
    self.writer.new_line();
    self.possible_new_line_save_point = None;
  }

  fn create_save_point(&self, _name: &'static str, next_node: Option<PrintItemPath>) -> &'a SavePoint<'a> {
    self.bump.alloc_save_point(SavePoint {
      #[cfg(debug_assertions)]
      name: _name,
      possible_new_line_save_point: self.possible_new_line_save_point,
      new_line_group_depth: self.new_line_group_depth,
      force_no_newlines_depth: self.force_no_newlines_depth,
      node: next_node,
      writer_state: self.writer.state(),
      look_ahead_condition_save_points: self.look_ahead_condition_save_points.clone(),
      look_ahead_line_number_save_points: self.look_ahead_line_number_save_points.clone(),
      look_ahead_column_number_save_points: self.look_ahead_column_number_save_points.clone(),
      look_ahead_is_start_of_line_save_points: self.look_ahead_is_start_of_line_save_points.clone(),
      look_ahead_indent_level_save_points: self.look_ahead_indent_level_save_points.clone(),
      look_ahead_line_start_column_number_save_points: self.look_ahead_line_start_column_number_save_points.clone(),
      look_ahead_line_start_indent_level_save_points: self.look_ahead_line_start_indent_level_save_points.clone(),
      next_node_stack: self.next_node_stack.clone(),
      resolutions_len: self.resolution_log.len(),
    })
  }

  #[inline]
  fn get_save_point_for_restoring_condition(&self, name: &'static str) -> &'a SavePoint<'a> {
    if let Some(save_point) = &self.resolving_save_point {
      save_point
    } else {
      self.create_save_point(name, self.current_node)
    }
  }

  fn mark_possible_new_line_if_able(&mut self) {
    if let Some(new_line_save_point) = &self.possible_new_line_save_point
      && self.new_line_group_depth > new_line_save_point.new_line_group_depth
    {
      return;
    }

    let next_node = self.current_node.as_ref().unwrap().get_next();
    self.possible_new_line_save_point = Some(self.create_save_point("newline", next_node));
  }

  #[inline]
  fn is_above_max_width(&self, offset: u32) -> bool {
    self.writer.column_number() + offset > self.max_width
  }

  fn update_state_to_save_point(&mut self, save_point: &'a SavePoint<'a>, is_for_new_line: bool) {
    self.writer.set_state(save_point.writer_state.clone());
    self.possible_new_line_save_point = if is_for_new_line { None } else { save_point.possible_new_line_save_point };

    while self.resolution_log.len() > save_point.resolutions_len {
      match self.resolution_log.pop().unwrap() {
        ResolutionState::Condition(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_conditions.insert(id, prev);
          } else {
            self.resolved_conditions.remove(&id);
          }
        }
        ResolutionState::LineNumberAnchor(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_line_number_anchors.insert(id, prev);
          } else {
            self.resolved_line_number_anchors.remove(id);
          }
        }
        ResolutionState::LineNumber(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_line_numbers.insert(id, prev);
          } else {
            self.resolved_line_numbers.remove(id);
          }
        }
        ResolutionState::ColumnNumber(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_column_numbers.insert(id, prev);
          } else {
            self.resolved_column_numbers.remove(id);
          }
        }
        ResolutionState::IsStartOfLine(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_is_start_of_lines.insert(id, prev);
          } else {
            self.resolved_is_start_of_lines.remove(id);
          }
        }
        ResolutionState::IndentLevel(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_indent_levels.insert(id, prev);
          } else {
            self.resolved_indent_levels.remove(id);
          }
        }
        ResolutionState::LineStartColumnNumber(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_line_start_column_numbers.insert(id, prev);
          } else {
            self.resolved_line_start_column_numbers.remove(id);
          }
        }
        ResolutionState::LineStartIndentLevel(id, prev) => {
          if let Some(prev) = prev {
            self.resolved_line_start_indent_levels.insert(id, prev);
          } else {
            self.resolved_line_start_indent_levels.remove(id);
          }
        }
        ResolutionState::StoredConditionSavePoint(id, prev) => {
          if let Some(prev) = prev {
            self.stored_condition_save_points.insert(id, prev);
          } else {
            self.stored_condition_save_points.remove(&id);
          }
        }
      }
    }

    self.current_node = save_point.node;
    self.new_line_group_depth = save_point.new_line_group_depth;
    self.force_no_newlines_depth = save_point.force_no_newlines_depth;
    self.look_ahead_condition_save_points.clone_from(&save_point.look_ahead_condition_save_points);
    self
      .look_ahead_line_number_save_points
      .clone_from(&save_point.look_ahead_line_number_save_points);
    self
      .look_ahead_column_number_save_points
      .clone_from(&save_point.look_ahead_column_number_save_points);
    self
      .look_ahead_is_start_of_line_save_points
      .clone_from(&save_point.look_ahead_is_start_of_line_save_points);
    self
      .look_ahead_indent_level_save_points
      .clone_from(&save_point.look_ahead_indent_level_save_points);
    self
      .look_ahead_line_start_column_number_save_points
      .clone_from(&save_point.look_ahead_line_start_column_number_save_points);
    self
      .look_ahead_line_start_indent_level_save_points
      .clone_from(&save_point.look_ahead_line_start_indent_level_save_points);
    self.next_node_stack = save_point.next_node_stack.clone();

    if is_for_new_line {
      self.write_new_line();
    }

    self.skip_moving_next = true;
  }

  #[inline]
  fn handle_signal(&mut self, signal: &Signal) {
    match signal {
      Signal::NewLine => {
        if self.allow_new_lines() {
          self.write_new_line()
        }
      }
      Signal::Tab => self.writer.tab(),
      Signal::ExpectNewLine => {
        // just always allow this for now since it's most likely a comment...
        self.writer.mark_expect_new_line();
        self.possible_new_line_save_point = None;
      }
      Signal::PossibleNewLine => {
        if self.allow_new_lines() {
          self.mark_possible_new_line_if_able()
        }
      }
      Signal::SpaceOrNewLine => {
        if self.allow_new_lines() {
          if self.is_above_max_width(1) {
            let optional_save_state = self.possible_new_line_save_point.take();
            if optional_save_state.is_none() {
              self.write_new_line();
            } else if let Some(save_state) = optional_save_state {
              if save_state.new_line_group_depth >= self.new_line_group_depth {
                self.write_new_line();
              } else {
                self.update_state_to_save_point(save_state, true);
              }
            }
          } else {
            self.mark_possible_new_line_if_able();
            self.writer.space_if_not_trailing();
          }
        } else {
          self.writer.space_if_not_trailing();
        }
      }
      Signal::QueueStartIndent => self.writer.queue_indent(),
      Signal::StartIndent => self.writer.start_indent(),
      Signal::FinishIndent => self.writer.finish_indent(),
      Signal::StartNewLineGroup => self.new_line_group_depth += 1,
      Signal::FinishNewLineGroup => self.new_line_group_depth -= 1,
      Signal::SingleIndent => self.writer.single_indent(),
      Signal::StartIgnoringIndent => self.writer.start_ignoring_indent(),
      Signal::FinishIgnoringIndent => self.writer.finish_ignoring_indent(),
      Signal::StartForceNoNewLines => self.force_no_newlines_depth += 1,
      Signal::FinishForceNoNewLines => self.force_no_newlines_depth -= 1,
      Signal::SpaceIfNotTrailing => self.writer.space_if_not_trailing(),
    }
  }

  #[inline]
  fn handle_anchor(&mut self, anchor: &Anchor) {
    match anchor {
      Anchor::LineNumber(anchor) => {
        let id = anchor.unique_id();
        let current_line_number = self.writer.line_number();
        if let Some(past_line_number) = self.resolved_line_number_anchors.get(id) {
          let difference = (current_line_number as isize) - (past_line_number as isize);
          if difference != 0 {
            let line_number_id = anchor.line_number_id();
            if let Some(value) = self.resolved_line_numbers.get(line_number_id) {
              let new_value = ((value as isize) + difference) as u32;
              self.resolution_log.push(ResolutionState::LineNumber(line_number_id, self.resolved_line_numbers.get(line_number_id)));
              self.resolved_line_numbers.insert(line_number_id, new_value);
            }
          }
        }
        self.resolution_log.push(ResolutionState::LineNumberAnchor(id, self.resolved_line_number_anchors.get(id)));
        self.resolved_line_number_anchors.insert(id, current_line_number);
      }
    }
  }

  #[inline]
  fn handle_targeted_info(&mut self, info: &Info) {
    match info {
      Info::LineNumber(line_number) => {
        let line_number_id = line_number.unique_id();
        let value = self.writer.line_number();
        self.resolution_log.push(ResolutionState::LineNumber(line_number_id, self.resolved_line_numbers.get(line_number_id)));
        self.resolved_line_numbers.insert(line_number_id, value);
        let option_save_point = self.look_ahead_line_number_save_points.remove(&line_number_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::LineNumber(line_number_id, self.resolved_line_numbers.get(line_number_id)));
          self.resolved_line_numbers.insert(line_number_id, value);
        }
      }
      Info::ColumnNumber(column_number) => {
        let column_number_id = column_number.unique_id();
        let value = self.writer.column_number();
        self.resolution_log.push(ResolutionState::ColumnNumber(column_number_id, self.resolved_column_numbers.get(column_number_id)));
        self.resolved_column_numbers.insert(column_number_id, value);
        let option_save_point = self.look_ahead_column_number_save_points.remove(&column_number_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::ColumnNumber(column_number_id, self.resolved_column_numbers.get(column_number_id)));
          self.resolved_column_numbers.insert(column_number_id, value);
        }
      }
      Info::IsStartOfLine(is_start_of_line) => {
        let is_start_of_line_id = is_start_of_line.unique_id();
        let value = self.writer.is_start_of_line();
        self.resolution_log.push(ResolutionState::IsStartOfLine(is_start_of_line_id, self.resolved_is_start_of_lines.get(is_start_of_line_id)));
        self.resolved_is_start_of_lines.insert(is_start_of_line_id, value);
        let option_save_point = self.look_ahead_is_start_of_line_save_points.remove(&is_start_of_line_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::IsStartOfLine(is_start_of_line_id, self.resolved_is_start_of_lines.get(is_start_of_line_id)));
          self.resolved_is_start_of_lines.insert(is_start_of_line_id, value);
        }
      }
      Info::IndentLevel(indent_level) => {
        let indent_level_id = indent_level.unique_id();
        let value = self.writer.indent_level();
        self.resolution_log.push(ResolutionState::IndentLevel(indent_level_id, self.resolved_indent_levels.get(indent_level_id)));
        self.resolved_indent_levels.insert(indent_level_id, value);
        let option_save_point = self.look_ahead_indent_level_save_points.remove(&indent_level_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::IndentLevel(indent_level_id, self.resolved_indent_levels.get(indent_level_id)));
          self.resolved_indent_levels.insert(indent_level_id, value);
        }
      }
      Info::LineStartColumnNumber(line_start_column_number) => {
        let line_start_column_number_id = line_start_column_number.unique_id();
        let value = self.writer.line_start_column_number();
        self.resolution_log.push(ResolutionState::LineStartColumnNumber(line_start_column_number_id, self.resolved_line_start_column_numbers.get(line_start_column_number_id)));
        self.resolved_line_start_column_numbers.insert(line_start_column_number_id, value);
        let option_save_point = self.look_ahead_line_start_column_number_save_points.remove(&line_start_column_number_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::LineStartColumnNumber(line_start_column_number_id, self.resolved_line_start_column_numbers.get(line_start_column_number_id)));
          self.resolved_line_start_column_numbers.insert(line_start_column_number_id, value);
        }
      }
      Info::LineStartIndentLevel(line_start_indent_level) => {
        let line_start_indent_level_id = line_start_indent_level.unique_id();
        let value = self.writer.line_start_indent_level();
        self.resolution_log.push(ResolutionState::LineStartIndentLevel(line_start_indent_level_id, self.resolved_line_start_indent_levels.get(line_start_indent_level_id)));
        self.resolved_line_start_indent_levels.insert(line_start_indent_level_id, value);
        let option_save_point = self.look_ahead_line_start_indent_level_save_points.remove(&line_start_indent_level_id);
        if let Some(save_point) = option_save_point {
          self.update_state_to_save_point(save_point, false);
          // Re-apply
          self.resolution_log.push(ResolutionState::LineStartIndentLevel(line_start_indent_level_id, self.resolved_line_start_indent_levels.get(line_start_indent_level_id)));
          self.resolved_line_start_indent_levels.insert(line_start_indent_level_id, value);
        }
      }
    }
  }

  #[inline]
  fn handle_condition_reevaluation(&mut self, condition_reevaluation: &ConditionReevaluation) {
    let condition_id = condition_reevaluation.condition_id;
    if let Some((condition, save_point)) = self.stored_condition_save_points.get(&condition_id).cloned()
      && let Some(past_condition_value) = self.resolved_conditions.get(&condition_id).and_then(|x| x.to_owned())
    {
      self.resolving_save_point.replace(save_point);
      let mut context = ConditionResolverContext::new(self, save_point.writer_state.writer_info(self.writer.indent_width()));
      let latest_condition_value = condition.resolve(&mut context);
      self.resolving_save_point.take();

      // Do not re-evaluate the condition if it's flipped back and forth a decent number of times.
      // If it hits the max number of times it can flip then an error will be logged.
      let should_reevaluate =
        self
          .infinite_reevaluation_protector
          .should_reevaluate(condition_reevaluation.condition_reevaluation_id, latest_condition_value, past_condition_value);
      if should_reevaluate {
        if let Some(latest_condition_value) = latest_condition_value {
          if latest_condition_value != past_condition_value {
            self.update_state_to_save_point(save_point, false);
          }
        } else {
          self.resolution_log.push(ResolutionState::Condition(condition_id, self.resolved_conditions.get(&condition_id).cloned()));
          self.resolved_conditions.remove(&condition_id);
        }
      }
    }
  }

  #[inline]
  fn handle_condition(&mut self, condition: &'a Condition, next_node: &Option<PrintItemPath>) {
    let condition_id = condition.unique_id();

    if condition.store_save_point {
      let save_point = self.get_save_point_for_restoring_condition(condition.name());
      self.resolution_log.push(ResolutionState::StoredConditionSavePoint(condition.unique_id(), self.stored_condition_save_points.get(&condition.unique_id()).cloned()));
      self.stored_condition_save_points.insert(condition.unique_id(), (condition, save_point));
    }

    let condition_value = condition.resolve(&mut ConditionResolverContext::new(self, self.get_writer_info()));
    if condition.is_stored {
      self.resolution_log.push(ResolutionState::Condition(condition_id, self.resolved_conditions.get(&condition_id).cloned()));
      self.resolved_conditions.insert(condition_id, condition_value);
    }

    if let Some(value) = condition_value {
      if let Some(save_point) = self.look_ahead_condition_save_points.remove(&condition_id) {
        self.update_state_to_save_point(save_point, false);
        // Re-apply
        self.resolution_log.push(ResolutionState::Condition(condition_id, self.resolved_conditions.get(&condition_id).cloned()));
        self.resolved_conditions.insert(condition_id, Some(value));
        return;
      }
    }

    if condition_value.is_some() && condition_value.unwrap() {
      if let Some(true_path) = condition.true_path {
        self.current_node = Some(true_path);
        if let Some(path) = next_node {
          self.next_node_stack.push(path, self.bump);
        }
        self.skip_moving_next = true;
      }
    } else if let Some(false_path) = condition.false_path {
      self.current_node = Some(false_path);
      if let Some(path) = next_node {
        self.next_node_stack.push(path, self.bump);
      }
      self.skip_moving_next = true;
    }
  }

  #[inline]
  fn handle_rc_path(&mut self, print_item_path: &PrintItemPath, next_node: &Option<PrintItemPath>) {
    if let Some(path) = next_node {
      self.next_node_stack.push(path, self.bump);
    }
    self.current_node = Some(print_item_path);
    self.skip_moving_next = true;
  }

  #[inline]
  fn handle_string(&mut self, text: &'a StringContainer) {
    #[cfg(debug_assertions)]
    self.validate_string(text.text);

    if self.possible_new_line_save_point.is_some() && self.is_above_max_width(text.char_count) && self.allow_new_lines() {
      let save_point = self.possible_new_line_save_point.take();
      self.update_state_to_save_point(save_point.unwrap(), true);
    } else {
      self.writer.write(text);
    }
  }

  #[inline]
  fn allow_new_lines(&self) -> bool {
    self.force_no_newlines_depth == 0
  }

  #[cfg(debug_assertions)]
  fn validate_string(&self, text: &str) {
    // The ir_helpers::gen_from_raw_string(...) helper function might be useful if you get either of these panics.
    if text.contains('\t') {
      panic!(
        "Debug panic! Found a tab in the string. Before sending the string to the printer it needs to be broken up and the tab sent as a PrintItem::Tab. {0}",
        text
      );
    }
    if text.contains('\n') {
      panic!(
        "Debug panic! Found a newline in the string. Before sending the string to the printer it needs to be broken up and the newline sent as a PrintItem::NewLine. {0}",
        text
      );
    }
  }

  #[cfg(debug_assertions)]
  fn verify_no_look_ahead_save_points(&self) {
    // The look ahead save points should be empty when printing is finished. If it's not
    // then that indicates that the generator tried to resolve a condition or info that was
    // never added to the print items. In this scenario, the look ahead hash maps will
    // be cloned when creating a save point and contain items that don't need to exist
    // in them thus having an unnecessary performance impact.
    let save_point = self
      .look_ahead_condition_save_points
      .values()
      .next()
      .or_else(|| self.look_ahead_line_number_save_points.values().next())
      .or_else(|| self.look_ahead_column_number_save_points.values().next())
      .or_else(|| self.look_ahead_is_start_of_line_save_points.values().next())
      .or_else(|| self.look_ahead_indent_level_save_points.values().next())
      .or_else(|| self.look_ahead_line_start_column_number_save_points.values().next())
      .or_else(|| self.look_ahead_line_start_indent_level_save_points.values().next());
    if let Some(save_point) = save_point {
      self.panic_for_save_point_existing(save_point)
    }
  }

  #[cfg(debug_assertions)]
  fn panic_for_save_point_existing(&self, save_point: &SavePoint<'a>) {
    panic!(
      concat!(
        "Debug panic! '{}' was never added to the print items in this scenario. This can ",
        "have slight performance implications in large files."
      ),
      save_point.name
    );
  }

  #[cfg(debug_assertions)]
  fn ensure_counts_zero(&self) {
    if self.new_line_group_depth != 0 {
      panic!(
        "Debug panic! The new line group depth was not zero after printing. {0}",
        self.new_line_group_depth
      );
    }
    if self.force_no_newlines_depth != 0 {
      panic!(
        "Debug panic! The force no newlines depth was not zero after printing. {0}",
        self.force_no_newlines_depth
      );
    }
    if self.writer.indentation_level() != 0 {
      panic!(
        "Debug panic! The writer indentation level was not zero after printing. {0}",
        self.writer.indentation_level()
      );
    }
    if self.writer.ignore_indent_count() != 0 {
      panic!(
        "Debug panic! The writer ignore indent count was not zero after printing. {0}",
        self.writer.ignore_indent_count()
      );
    }
  }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::thread_state;

    #[test]
    fn it_should_rollback_resolutions() {
        thread_state::with_bump_allocator(|bump| {
            let options = PrinterOptions {
                max_width: 10,
                indent_width: 2,
                #[cfg(feature = "tracing")]
                enable_tracing: false,
            };
            let mut printer = Printer::new(bump, None, options);

            let condition_id = 1;

            // 1. Initial state: not in cache
            assert!(printer.resolved_conditions.get(&condition_id).is_none());

            // 2. Create save point
            let save_point = printer.create_save_point("test", None);

            // 3. Modify cache
            printer.resolution_log.push(ResolutionState::Condition(condition_id, None));
            printer.resolved_conditions.insert(condition_id, Some(true));

            assert_eq!(printer.resolved_conditions.get(&condition_id).unwrap().unwrap(), true);

            // 4. Restore save point
            printer.update_state_to_save_point(save_point, false);

            // 5. Cache should be rolled back
            assert!(printer.resolved_conditions.get(&condition_id).is_none());
        });
    }

    #[test]
    fn it_should_rollback_info_resolutions() {
        thread_state::with_bump_allocator(|bump| {
            // Allocate an ID first so the printer has capacity for it
            let info_id = thread_state::next_line_number_id();

            let options = PrinterOptions {
                max_width: 10,
                indent_width: 2,
                #[cfg(feature = "tracing")]
                enable_tracing: false,
            };
            let mut printer = Printer::new(bump, None, options);

            // Initial state
            assert!(printer.resolved_line_numbers.get(info_id).is_none());

            // Create save point
            let save_point = printer.create_save_point("test", None);

            // Modify cache
            printer.resolution_log.push(ResolutionState::LineNumber(info_id, None));
            printer.resolved_line_numbers.insert(info_id, 10);

            assert_eq!(printer.resolved_line_numbers.get(info_id).unwrap(), 10);

            // Restore save point
            printer.update_state_to_save_point(save_point, false);

            // Cache should be rolled back
            assert!(printer.resolved_line_numbers.get(info_id).is_none());
        });
    }

    #[test]
    fn it_should_rollback_to_previous_value() {
        thread_state::with_bump_allocator(|bump| {
            let options = PrinterOptions {
                max_width: 10,
                indent_width: 2,
                #[cfg(feature = "tracing")]
                enable_tracing: false,
            };
            let mut printer = Printer::new(bump, None, options);

            let condition_id = 1;

            // Set initial value
            printer.resolved_conditions.insert(condition_id, Some(false));

            // Create save point
            let save_point = printer.create_save_point("test", None);

            // Modify cache
            printer.resolution_log.push(ResolutionState::Condition(condition_id, Some(Some(false))));
            printer.resolved_conditions.insert(condition_id, Some(true));

            assert_eq!(printer.resolved_conditions.get(&condition_id).unwrap().unwrap(), true);

            // Restore save point
            printer.update_state_to_save_point(save_point, false);

            // Cache should be rolled back to initial value
            assert_eq!(printer.resolved_conditions.get(&condition_id).unwrap().unwrap(), false);
        });
    }

    #[test]
    fn it_should_rollback_all_info_types() {
        thread_state::with_bump_allocator(|bump| {
            let anchor_id = thread_state::next_line_number_anchor_id();
            let line_id = thread_state::next_line_number_id();
            let col_id = thread_state::next_column_number_id();
            let is_start_id = thread_state::next_is_start_of_line_id();
            let indent_id = thread_state::next_indent_level_id();
            let line_start_col_id = thread_state::next_line_start_column_number_id();
            let line_start_indent_id = thread_state::next_line_start_indent_level_id();

            let options = PrinterOptions {
                max_width: 80,
                indent_width: 2,
                #[cfg(feature = "tracing")]
                enable_tracing: false,
            };
            let mut printer = Printer::new(bump, None, options);

            let save_point = printer.create_save_point("test", None);

            printer.resolution_log.push(ResolutionState::LineNumberAnchor(anchor_id, None));
            printer.resolved_line_number_anchors.insert(anchor_id, 1);
            printer.resolution_log.push(ResolutionState::LineNumber(line_id, None));
            printer.resolved_line_numbers.insert(line_id, 2);
            printer.resolution_log.push(ResolutionState::ColumnNumber(col_id, None));
            printer.resolved_column_numbers.insert(col_id, 3);
            printer.resolution_log.push(ResolutionState::IsStartOfLine(is_start_id, None));
            printer.resolved_is_start_of_lines.insert(is_start_id, true);
            printer.resolution_log.push(ResolutionState::IndentLevel(indent_id, None));
            printer.resolved_indent_levels.insert(indent_id, 4);
            printer.resolution_log.push(ResolutionState::LineStartColumnNumber(line_start_col_id, None));
            printer.resolved_line_start_column_numbers.insert(line_start_col_id, 5);
            printer.resolution_log.push(ResolutionState::LineStartIndentLevel(line_start_indent_id, None));
            printer.resolved_line_start_indent_levels.insert(line_start_indent_id, 6);

            printer.update_state_to_save_point(save_point, false);

            assert!(printer.resolved_line_number_anchors.get(anchor_id).is_none());
            assert!(printer.resolved_line_numbers.get(line_id).is_none());
            assert!(printer.resolved_column_numbers.get(col_id).is_none());
            assert!(printer.resolved_is_start_of_lines.get(is_start_id).is_none());
            assert!(printer.resolved_indent_levels.get(indent_id).is_none());
            assert!(printer.resolved_line_start_column_numbers.get(line_start_col_id).is_none());
            assert!(printer.resolved_line_start_indent_levels.get(line_start_indent_id).is_none());
        });
    }

    #[test]
    fn it_should_handle_nested_rollback() {
        thread_state::with_bump_allocator(|bump| {
            let options = PrinterOptions {
                max_width: 80,
                indent_width: 2,
                #[cfg(feature = "tracing")]
                enable_tracing: false,
            };
            let mut printer = Printer::new(bump, None, options);

            let id1 = 1;
            let id2 = 2;

            let sp1 = printer.create_save_point("sp1", None);
            printer.resolution_log.push(ResolutionState::Condition(id1, None));
            printer.resolved_conditions.insert(id1, Some(true));

            let sp2 = printer.create_save_point("sp2", None);
            printer.resolution_log.push(ResolutionState::Condition(id2, None));
            printer.resolved_conditions.insert(id2, Some(false));

            assert_eq!(printer.resolved_conditions.get(&id1).unwrap().unwrap(), true);
            assert_eq!(printer.resolved_conditions.get(&id2).unwrap().unwrap(), false);

            // Rollback to sp2
            printer.update_state_to_save_point(sp2, false);
            assert_eq!(printer.resolved_conditions.get(&id1).unwrap().unwrap(), true);
            assert!(printer.resolved_conditions.get(&id2).is_none());

            // Rollback to sp1
            printer.update_state_to_save_point(sp1, false);
            assert!(printer.resolved_conditions.get(&id1).is_none());
            assert!(printer.resolved_conditions.get(&id2).is_none());
        });
    }
}
