//  Copyright 2022 Lance Authors
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.

#pragma once

#include <arrow/dataset/scanner.h>
#include <arrow/record_batch.h>
#include <arrow/result.h>

#include <memory>
#include <mutex>
#include <tuple>
#include <vector>

#include "lance/arrow/fragment.h"
#include "lance/io/exec/base.h"

namespace lance::format {
class Schema;
}

namespace lance::io {
class FileReader;
}

namespace lance::io::exec {

/// Leaf scan node.
class Scan : public ExecNode {
 public:
  /// Factory method.
  ///
  /// \param readers a vector of the tuples of `[reader, schema]`, including opened file reader
  ///                and projection schema.
  /// \param batch_size batch size.
  /// \param executor executor to run parallel jobs.
  /// \return a Scan node if succeed.
  static ::arrow::Result<std::unique_ptr<Scan>> Make(
      const std::vector<lance::arrow::LanceFragment::FileReaderWithSchema>& readers,
      int64_t batch_size,
      ::arrow::internal::Executor* executor = ::arrow::internal::GetCpuThreadPool());

  Scan() = delete;

  ~Scan() override = default;

  constexpr Type type() const override { return Type::kScan; }

  /// Returns the next available batch in the file. Or returns nullptr if EOF.
  ::arrow::Result<ScanBatch> Next() override;

  ::arrow::Result<std::shared_ptr<::arrow::RecordBatch>> Take(
      int32_t batch_id, const std::shared_ptr<::arrow::Int32Array>& indices);

  /// Debug String
  std::string ToString() const override;

  /// Seek to a particular row.
  ///
  /// It is used by LIMIT to advance the starting offset, allows to skip IOs.
  ///
  /// \param offset the number of rows to seek forward.
  ::arrow::Status Seek(int32_t offset);

 private:
  /// Constructor
  ///
  /// \param readers A vector of opened readers with the projected schema.
  /// \param batch_size scan batch size.
  /// \param executor executor to run parallel jobs.
  Scan(const std::vector<lance::arrow::LanceFragment::FileReaderWithSchema>& readers,
       int64_t batch_size,
       ::arrow::internal::Executor* executor);

  std::vector<lance::arrow::LanceFragment::FileReaderWithSchema> readers_;
  const int64_t batch_size_;
  ::arrow::internal::Executor* executor_;

  /// Keep track of the progress.
  std::mutex lock_;
  int32_t current_batch_id_ = 0;
  /// Offset in the batch.
  int32_t current_offset_ = 0;
  ///
  int32_t current_batch_page_length_ = 0;
};

}  // namespace lance::io::exec
